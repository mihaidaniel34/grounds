#!/usr/bin/env python3
"""History replay: `grounds changed` over the real commits of a project.

The work tree walks the last N commits of the checkout's main line (first
parents), oldest first, each step moving from a commit's parent to the commit
as a set of edits would. After each step `grounds changed --base <parent>` is
asked twice: through the daemon, which has followed every step so far, and
with --no-daemon, which loads the project from scratch.

  same      the two answers must be equal, field for field: what is reported
            after edits is what a fresh run reports for the result.
  compiles  a commit on a project's main line compiled, so a finding of a
            rule that claims it would not, or a place "broken" in another
            file, is a false report. (A main line has the odd commit that
            did not compile; such a report is to be read, then listed in
            --known.)

Commits that change no Java file are skipped. The checkout is put back on the
commit it was at; uncommitted changes to tracked files are refused. With a
shallow checkout only the commits it has are replayed.

  python3 -I scripts/replay.py DIR [--commits 30] [--grounds target/release/grounds]
      [--known FILE] [--out report.json]

--known lists `commit path:line rule` lines of reports already looked at.
Exit status 1 if the daemon and a fresh run disagreed, or a compile-break
report is not in --known.
"""

import argparse
import json
import os
import subprocess
import sys

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
    r = json.loads(p.stdout)
    return {k: sorted(json.dumps(x, sort_keys=True) for x in r.get(k, [])) for k in ("findings", "pending")}


def where(x):
    f = json.loads(x)
    return f"{f.get('path')}:{f.get('line')} {f.get('rule', 'broken-by-change')}"


def summary(path):
    with open(path) as fh:
        st = json.load(fh)["stats"]
    print(f"replay: {st['commits']} commits, daemon differs from a fresh run on {st['differs']}, "
          f"compile-break reports on {st['reported']} ({st['known']} known)")
    return 0


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--summary":
        return summary(sys.argv[2])
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--commits", type=int, default=30)
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
    line = git(d, "rev-list", "--first-parent", "-n", str(a.commits + 1), head).split()
    line.reverse()  # oldest first; line[0] is only a starting point
    steps, stats = [], {"commits": 0, "skipped": 0, "differs": 0, "reported": 0, "known": 0}
    failed = False
    subprocess.run([grounds, "daemon", "--root", d, "--stop"], capture_output=True)
    try:
        git(d, "checkout", "-q", "--detach", line[0])
        changed(grounds, d, None, True)  # the daemon loads the project as of the first commit
        for parent, commit in zip(line, line[1:]):
            touched = git(d, "diff", "--name-only", parent, commit, "--", "*.java").split()
            git(d, "checkout", "-q", "--detach", commit)
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
                only_warm = [where(x) for k in warm for x in warm[k] if x not in fresh[k]]
                only_fresh = [where(x) for k in fresh for x in fresh[k] if x not in warm[k]]
                step["daemon-only"], step["fresh-only"] = only_warm, only_fresh
                print(f"DAEMON DIFFERS from a fresh run at {commit[:12]} ({len(touched)} files):")
                for w in only_warm[:10]:
                    print(f"    daemon only: {w}")
                for w in only_fresh[:10]:
                    print(f"    fresh only:  {w}")
            breaks = [where(x) for x in fresh["pending"]]
            breaks += [where(x) for x in fresh["findings"] if json.loads(x)["rule"] in COMPILE_RULES]
            if breaks:
                stats["reported"] += 1
                new = [b for b in breaks if f"{commit[:12]} {b}" not in known]
                step["compile-breaks"] = breaks
                if new:
                    failed = True
                    print(f"COMPILE BREAK reported at {commit[:12]}, a commit of the main line:")
                    for b in new[:10]:
                        print(f"    {commit[:12]} {b}")
                else:
                    stats["known"] += 1
            steps.append(step)
    finally:
        subprocess.run([grounds, "daemon", "--root", d, "--stop"], capture_output=True)
        git(d, "checkout", "-q", "--detach", head)
    print(f"{project}: {stats['commits']} commits replayed ({stats['skipped']} without Java changes), "
          f"daemon differs on {stats['differs']}, compile-break reports on {stats['reported']} ({stats['known']} known)")
    if a.out:
        with open(a.out, "w") as fh:
            json.dump({"project": project, "head": head, "stats": stats, "steps": steps}, fh, indent=1)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
