#!/usr/bin/env python3
"""History replay: `grounds changed` over the real commits of a project.

The work tree walks the checkout's main line (first parents), oldest first,
from before the last N commits that change a Java file up to the checkout's
commit, each step moving from a commit's parent to the commit as a set of
edits would. After each step `grounds changed --base <parent>` is
asked twice: through the daemon, which has followed every step so far, and
with --no-daemon, which loads the project from scratch.

  same      the two answers must be equal, field for field: what is reported
            after edits is what a fresh run reports for the result.
  compiles  a commit on a project's main line compiled and its application
            started, so a finding of a rule that claims it would not, or a
            place in another file "broken" by the commit (a call that no
            longer resolves, a bean that is no longer found), is a false
            report. (A main line has the odd commit that did not build; such
            a report is to be read, then listed in --known.)

Commits in between that change no Java file are checked out but not asked
about (the daemon still has to follow what they do to build files). With
--budget the walk stops asking once that many seconds have passed, and says
how far it got. The checkout is put back on the
branch or commit it was at. Uncommitted changes to tracked files are refused,
and a checkout that would overwrite an ignored file stops the run; all the
same, run it on a checkout made for the purpose. With a shallow checkout only
the commits it has are replayed.

  python3 -I scripts/replay.py DIR [--commits 30] [--budget SECONDS]
      [--grounds target/release/grounds] [--known FILE] [--out report.json]

--known lists `project commit path:line rule` lines of reports already
looked at (commit: its first 12 characters; `#` starts a comment).
Exit status 1 if the daemon and a fresh run disagreed, or a compile-break
report is not in --known.
"""

import argparse
import json
import os
import subprocess
import sys
import time

COMPILE_RULES = {"unresolved-method", "unresolved-constructor", "unresolved-field", "unresolved-static-import",
                 "missing-jdk-import", "override-matches-nothing", "unimplemented-abstract-method",
                 "incompatible-assignment", "removed-class-still-used", "breaks-other-files"}


def git(d, *args, check=True):
    return subprocess.run(["git", "-C", d, *args], capture_output=True, text=True, check=check).stdout


def changed(grounds, d, base, daemon):
    cmd = [grounds, "changed", "--root", d, "--format", "json", "--exit-zero", "--max", "100000",
           "--min-severity", "info", "--no-baseline", "--color", "never"]
    if base:
        cmd += ["--base", base]
    if not daemon:
        cmd.append("--no-daemon")
    p = subprocess.run(cmd, capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if p.returncode != 0:
        raise RuntimeError(f"grounds changed failed ({p.returncode}): {p.stderr[-2000:]}")
    # the whole answer: lists in a fixed order, everything else as it is
    return {k: sorted(json.dumps(x, sort_keys=True) for x in v) if isinstance(v, list) else v
            for k, v in json.loads(p.stdout).items()}


def where(x):
    f = json.loads(x)
    if not isinstance(f, dict):
        return str(f)
    return f"{f.get('path')}:{f.get('line')} {f.get('rule', 'broken-by-change')}"


def difference(a, b):
    """What answer `a` has and `b` lacks, as lines to print."""
    out = []
    for k, v in a.items():
        if isinstance(v, list):
            out += [f"{k}: {where(x)}" for x in v if x not in (b.get(k) or [])]
        elif v != b.get(k):
            out.append(f"{k}: {v}")
    return out


def summary(path):
    with open(path) as fh:
        st = json.load(fh)["stats"]
    cut = f" (stopped by the time budget, {st['left']} left)" if st.get("left") else ""
    print(f"replay: {st['commits']} commits{cut}, daemon differs from a fresh run on {st['differs']}, "
          f"compile-break reports on {st['reported']} ({st['known']} known)")
    return 0


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--summary":
        return summary(sys.argv[2])
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--commits", type=int, default=30, help="commits that change a Java file")
    ap.add_argument("--budget", type=float, help="stop asking after this many seconds")
    ap.add_argument("--grounds", default=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                                                      "target", "release", "grounds"))
    ap.add_argument("--known")
    ap.add_argument("--out")
    a = ap.parse_args()
    d = os.path.abspath(a.dir)
    grounds = os.path.abspath(a.grounds)
    project = os.path.basename(d)
    if git(d, "status", "--porcelain", "--untracked-files=no").strip():
        sys.exit(f"{d}: uncommitted changes to tracked files; refusing to move the checkout")
    known = set()
    if a.known and os.path.exists(a.known):
        with open(a.known) as fh:
            known = {" ".join(l.split()) for l in fh if l.strip() and not l.startswith("#")}
    head = git(d, "rev-parse", "HEAD").strip()
    # the branch to go back to, if the checkout is on one
    back = git(d, "symbolic-ref", "-q", "--short", "HEAD", check=False).strip() or head
    # ignored files are otherwise overwritten without a word by a commit that tracked them
    move = ["checkout", "-q", "--no-overwrite-ignore", "--detach"]
    # the main line, newest first, back to the parent of the Nth commit with Java changes
    line = git(d, "rev-list", "--first-parent", head).split()
    java = lambda parent, commit: git(d, "diff", "--name-only", parent, commit, "--", "*.java").split()
    wanted, start = 0, len(line) - 1
    for i in range(len(line) - 1):
        if java(line[i + 1], line[i]):
            wanted += 1
            if wanted == a.commits:
                start = i + 1
                break
    line = line[:start + 1]
    line.reverse()  # oldest first; line[0] is only a starting point
    steps, stats = [], {"commits": 0, "skipped": 0, "differs": 0, "reported": 0, "known": 0, "left": 0}
    began = time.monotonic()
    failed = False
    subprocess.run([grounds, "daemon", "--root", d, "--stop"], capture_output=True)
    try:
        git(d, *move, line[0])
        changed(grounds, d, None, True)  # the daemon loads the project as of the first commit
        for parent, commit in zip(line, line[1:]):
            touched = java(parent, commit)
            if touched and a.budget is not None and time.monotonic() - began > a.budget:
                stats["left"] += 1
                continue
            git(d, *move, commit)
            if not touched:
                stats["skipped"] += 1
                continue
            stats["commits"] += 1
            warm = changed(grounds, d, parent, True)
            fresh = changed(grounds, d, parent, False)
            step = {"commit": commit, "files": len(touched)}
            if warm != fresh:
                stats["differs"] += 1
                failed = True
                only_warm, only_fresh = difference(warm, fresh), difference(fresh, warm)
                step["daemon-only"], step["fresh-only"] = only_warm, only_fresh
                print(f"DAEMON DIFFERS from a fresh run at {commit[:12]} ({len(touched)} files):")
                for w in only_warm[:10]:
                    print(f"    daemon only: {w}")
                for w in only_fresh[:10]:
                    print(f"    fresh only:  {w}")
            breaks = [where(x) for x in fresh.get("pending", [])]
            breaks += [where(x) for x in fresh.get("findings", []) if json.loads(x)["rule"] in COMPILE_RULES]
            if breaks:
                stats["reported"] += 1
                new = [b for b in breaks if f"{project} {commit[:12]} {b}" not in known]
                step["compile-breaks"] = breaks
                if new:
                    failed = True
                    print(f"COMPILE BREAK reported at {commit[:12]}, a commit of the main line:")
                    for b in new[:10]:
                        print(f"    {project} {commit[:12]} {b}")
                else:
                    stats["known"] += 1
            steps.append(step)
    finally:
        subprocess.run([grounds, "daemon", "--root", d, "--stop"], capture_output=True)
        git(d, "checkout", "-q", "--no-overwrite-ignore", *(["--detach", head] if back == head else [back]))
    if stats["left"]:
        print(f"{project}: time budget of {a.budget:.0f} s used up, {stats['left']} commits with Java changes not asked about")
    print(f"{project}: {stats['commits']} commits replayed ({stats['skipped']} without Java changes), "
          f"daemon differs on {stats['differs']}, compile-break reports on {stats['reported']} ({stats['known']} known)")
    if a.out:
        with open(a.out, "w") as fh:
            json.dump({"project": project, "head": head, "stats": stats, "steps": steps}, fh, indent=1)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
