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
about (the daemon still has to follow what they do to build files).

Then files go and come back, as an edit that deletes a file and a `git
reset` that restores it do: every tracked Java file is deleted, with the
directories that are left empty, and both are asked (`changed`); git
restores them, and both are asked about the whole walk as one change
(`changed`). Restored, a file equals HEAD and is in no
list of changed files, so only a daemon that finds files by itself holds
them again. A daemon that does not has most of the project unknown, which a
main line that compiles may not show; so then a public method of a class
that many files import, the one they call the most, is renamed and both are
asked once more: a fresh run reports the calls as broken, and so must the
daemon.

With --budget SECONDS the newest commit is asked about once first, to see
what a step costs on this project, and N is lowered to what fits: the walk
is of the newest commits either way, and every commit chosen is asked
about, also when the walk then takes longer than estimated. A single
`grounds changed` that takes longer than --timeout (default 900 s) is a
failure. The checkout is put back on the
branch or commit it was at. Uncommitted changes to tracked files are refused,
and a checkout that would overwrite an ignored file stops the run; all the
same, run it on a checkout made for the purpose. With a shallow checkout only
the commits it has are replayed.

  python3 -I scripts/replay.py DIR [--commits 30] [--budget SECONDS] [--timeout SECONDS]
      [--grounds target/release/grounds] [--known FILE] [--out report.json]

--known lists `project commit path:line rule` lines of reports already
looked at (commit: its first 12 characters; `#` starts a comment).
Exit status 1 if the daemon and a fresh run disagreed, or a compile-break
report is not in --known.
"""

import argparse
import itertools
import json
import math
import os
import re
import subprocess
import sys
import time

COMPILE_RULES = {"unresolved-method", "unresolved-constructor", "unresolved-field", "unresolved-static-import",
                 "missing-jdk-import", "override-matches-nothing", "unimplemented-abstract-method",
                 "incompatible-assignment", "removed-class-still-used", "breaks-other-files"}


def git(d, *args, check=True):
    return subprocess.run(["git", "-C", d, *args], capture_output=True, text=True, check=check).stdout


TIMEOUT = 900.0


def changed(grounds, d, base, daemon):
    cmd = [grounds, "changed", "--root", d, "--format", "json", "--exit-zero", "--max", "100000",
           "--min-severity", "info", "--no-baseline", "--color", "never"]
    if base:
        cmd += ["--base", base]
    if not daemon:
        cmd.append("--no-daemon")
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"grounds changed gave no answer in {TIMEOUT:.0f} s ({'daemon' if daemon else 'fresh'}, base {base})")
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


# a public method declared on one line: its name
DECLARATION = re.compile(r"^\s*public\s+(?:(?:static|final|synchronized|abstract|default)\s+)*(?:<[^>]*>\s+)?[\w.$<>\[\], ?]+\s+(\w+)\s*\(")


def rename_probes(d, files):
    """Methods to rename so that other files stop compiling, best first: per
    project class among the 20 that the most files import by name, its public
    method declared once that those files call the most (`Class.name(` for a
    static one, `.name(` else). Yields (file, line index, name, (start, end)
    of the name in that line)."""
    importers = {}
    for l in git(d, "grep", "-E", r"^import [A-Za-z_][A-Za-z0-9_.]*;", "--", "*.java", check=False).splitlines():
        f, _, line = l.partition(":")
        importers.setdefault(line[len("import "):].strip().rstrip(";"), []).append(f)
    by_name = {}
    for f in files:
        by_name.setdefault(os.path.basename(f)[:-len(".java")], []).append(f)
    for n, users in sorted(importers.items(), key=lambda x: (-len(x[1]), x[0]))[:20]:
        suffix = n.replace(".", "/") + ".java"
        f = next((f for f in by_name.get(n.rsplit(".", 1)[-1], []) if f == suffix or f.endswith("/" + suffix)), None)
        if f is None:
            continue
        with open(os.path.join(d, f), encoding="utf-8", errors="surrogateescape", newline="") as fh:
            lines = fh.read().splitlines(keepends=True)
        declared = {}
        for i, l in enumerate(lines):
            m = DECLARATION.match(l)
            if m:
                declared.setdefault(m.group(1), []).append((i, m.span(1)))
        once = {k: v[0][0] for k, v in declared.items() if len(v) == 1}
        span = {k: v[0][1] for k, v in declared.items() if len(v) == 1}
        if not once:
            continue
        simple = n.rsplit(".", 1)[-1]
        call = {k: f"{simple}.{k}(" if re.search(r"\bstatic\b", lines[i].split("(")[0]) else f".{k}(" for k, i in once.items()}
        calls = dict.fromkeys(once, 0)
        for u in users:
            try:
                with open(os.path.join(d, u), encoding="utf-8", errors="replace") as fh:
                    text = fh.read()
            except OSError:
                continue
            for k in calls:
                calls[k] += text.count(call[k])
        name = max(sorted(calls), key=lambda k: calls[k])
        if calls[name]:
            yield f, once[name], name, span[name]


def summary(path):
    with open(path) as fh:
        st = json.load(fh)["stats"]
    print(f"replay: {st['commits']} commits, daemon differs from a fresh run on {st['differs']}, "
          f"compile-break reports on {st['reported']} ({st['known']} known)")
    return 0


def main():
    global TIMEOUT
    if len(sys.argv) == 3 and sys.argv[1] == "--summary":
        return summary(sys.argv[2])
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--commits", type=int, default=30, help="commits that change a Java file")
    ap.add_argument("--budget", type=float, help="seconds of asking to size the walk for")
    ap.add_argument("--timeout", type=float, default=TIMEOUT, help="seconds one `grounds changed` may take")
    ap.add_argument("--grounds", default=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                                                      "target", "release", "grounds"))
    ap.add_argument("--known")
    ap.add_argument("--out")
    a = ap.parse_args()
    if a.commits < 1:
        ap.error("--commits must be at least 1")
    if a.budget is not None and not (math.isfinite(a.budget) and a.budget > 0):
        ap.error("--budget must be a positive number of seconds")
    # (infinity passes `> 0` and then overflows in subprocess's polling; NaN
    # fails every comparison and would never time out)
    if not (math.isfinite(a.timeout) and a.timeout > 0):
        ap.error("--timeout must be a positive number of seconds")
    TIMEOUT = a.timeout
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
    restore = lambda: git(d, "checkout", "-q", "--no-overwrite-ignore", *(["--detach", head] if back == head else [back]))
    steps_back = []
    for i in range(len(line) - 1):  # no further back than needed: a full clone has a long history
        if java(line[i + 1], line[i]):
            steps_back.append(i)
            if len(steps_back) == a.commits:
                break
    fit = len(steps_back)
    if a.budget is not None and steps_back:
        # what the newest step costs from scratch; the daemon's answer costs about as much
        i = steps_back[0]
        try:
            git(d, *move, line[i])
            t = time.monotonic()
            changed(grounds, d, line[i + 1], False)
            cost = 2 * (time.monotonic() - t)
        except BaseException:
            restore()  # the walk below, which puts the checkout back, has not begun
            raise
        # (with a fifth to spare: the first steps also load the project)
        fit = max(1, min(fit, int(0.8 * a.budget / cost))) if cost > 0 else fit
        if fit < len(steps_back):
            print(f"{project}: a step takes about {cost:.0f} s, so the last {fit} of {len(steps_back)} commits with Java changes are replayed")
    start = steps_back[fit - 1] + 1 if steps_back else 0
    line = line[:start + 1]
    line.reverse()  # oldest first; line[0] is only a starting point
    steps, stats = [], {"commits": 0, "skipped": 0, "differs": 0, "reported": 0, "known": 0}
    failed = False

    def compare(step, what, base):
        """Ask the daemon and a fresh run; a difference fails the run."""
        nonlocal failed
        t = time.monotonic()
        warm = changed(grounds, d, base, True)
        step["daemon-seconds"] = round(time.monotonic() - t, 1)
        fresh = changed(grounds, d, base, False)
        step["fresh-seconds"] = round(time.monotonic() - t - step["daemon-seconds"], 1)
        if warm != fresh:
            stats["differs"] += 1
            failed = True
            only_warm, only_fresh = difference(warm, fresh), difference(fresh, warm)
            step["daemon-only"], step["fresh-only"] = only_warm, only_fresh
            print(f"DAEMON DIFFERS from a fresh run {what}:")
            for w in only_warm[:10]:
                print(f"    daemon only: {w}")
            for w in only_fresh[:10]:
                print(f"    fresh only:  {w}")
        return fresh

    dirty = False  # the work tree holds edits of this script
    subprocess.run([grounds, "daemon", "--root", d, "--stop"], capture_output=True)
    try:
        git(d, *move, line[0])
        # The first asking starts the daemon and is answered without it: the
        # daemon loads in the background. Wait until it answers itself, so
        # that the walk asks the daemon and no checkout lands in its load.
        changed(grounds, d, None, True)
        # (each asking gets what is left of the time: a daemon that has its
        # socket and is still loading keeps the status command waiting)
        deadline = time.monotonic() + TIMEOUT
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise RuntimeError(f"the daemon did not come up in {TIMEOUT:.0f} s")
            try:
                status = subprocess.run([grounds, "daemon", "--root", d, "--status"], capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=left).stdout
            except subprocess.TimeoutExpired:
                status = ""
            if status.startswith("running:") and time.monotonic() <= deadline:
                break
            time.sleep(max(0.0, min(0.2, deadline - time.monotonic())))
        for parent, commit in zip(line, line[1:]):
            touched = java(parent, commit)
            git(d, *move, commit)
            if not touched:
                stats["skipped"] += 1
                continue
            stats["commits"] += 1
            step = {"commit": commit, "files": len(touched)}
            fresh = compare(step, f"at {commit[:12]} ({len(touched)} files)", parent)
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
        # files that go and come back (see above)
        t = time.monotonic()
        files = [f for f in git(d, "ls-files", "-z", "--", "*.java").split("\0") if f]
        dirty = True
        emptied = set()
        for f in files:
            p = os.path.join(d, f)
            if os.path.isfile(p):
                os.remove(p)
                emptied.add(os.path.dirname(p))
        for x in sorted(emptied, key=len, reverse=True):
            while x.startswith(d + os.sep):
                try:
                    os.rmdir(x)
                except OSError:
                    break
                x = os.path.dirname(x)
        step = {"commit": "deleted", "files": len(files)}
        compare(step, f"with the {len(files)} tracked Java files deleted", None)
        steps.append(step)
        git(d, "checkout", "-q", "--", ".")
        dirty = False
        step = {"commit": "restored", "files": len(files)}
        compare(step, "with them restored", None)
        steps.append(step)
        # (the next candidate while a fresh run reports no broken place: at most three)
        for f, i, name, (begin, end) in itertools.islice(rename_probes(d, files), 3):
            path = os.path.join(d, f)
            with open(path, encoding="utf-8", errors="surrogateescape", newline="") as fh:
                lines = fh.read().splitlines(keepends=True)
            # (the name where the declaration was matched, not its first
            # `name(`: `run ()` has none; a file that changed since is skipped)
            if lines[i][begin:end] != name:
                print(f"{project}: {name} is not where it was found in {f}; the next candidate")
                continue
            lines[i] = lines[i][:begin] + f"{name}ZzReplay" + lines[i][end:]
            dirty = True  # (the checkout is put back below, or by `finally`)
            with open(path, "w", encoding="utf-8", errors="surrogateescape", newline="") as fh:
                fh.write("".join(lines))
            step = {"commit": "renamed", "files": 1, "file": f, "method": name}
            fresh = compare(step, f"with {name} renamed in {f}", None)
            step["pending"] = len(fresh.get("pending", []))
            steps.append(step)
            git(d, "checkout", "-q", "--", ".")
            dirty = False
            if step["pending"] or "daemon-only" in step:
                break
        step["seconds"] = round(time.monotonic() - t, 1)
        print(f"{project}: files deleted, restored and a method renamed, each asked both ways, in {step['seconds']:.0f} s")
    finally:
        subprocess.run([grounds, "daemon", "--root", d, "--stop"], capture_output=True)
        if dirty:
            git(d, "checkout", "-q", "--", ".")
        restore()
    print(f"{project}: {stats['commits']} commits replayed ({stats['skipped']} without Java changes), "
          f"daemon differs on {stats['differs']}, compile-break reports on {stats['reported']} ({stats['known']} known)")
    if a.out:
        with open(a.out, "w") as fh:
            json.dump({"project": project, "head": head, "stats": stats, "steps": steps}, fh, indent=1)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
