#!/usr/bin/env python3
"""What an agent is told after each real commit: every finding, not only breaks.

scripts/replay.py asks `grounds changed --base <parent>` after each of a
project's last commits that change Java, but keeps only the compile and
wiring breaks. This script asks the same question for the same commits and
keeps the whole answer an agent would read:

  grounds changed --root DIR --base <parent> --format json --no-daemon
      --no-baseline --max 100000 --min-severity warning --exit-zero

that is, the findings on the lines the commit changed (warnings and errors)
and the `pending` list (places in other files the commit broke), with the
number of files whose declarations the commit changed. The commits
are the steps of a replay.py --out report (--from), or else, as replay.py
picks them, the last --commits first-parent commits that change a Java file.
<parent> is the commit's first parent. Each commit is checked out on its own
(no daemon follows the walk); the checkout is put back where it was.
Uncommitted changes to tracked files are refused.

  python3 -I scripts/replay_findings.py DIR [--from replay.json | --commits 15]
      [--grounds target/release/grounds] [--timeout 900] --out findings.json
  python3 -I scripts/replay_findings.py --summary a.json b.json ...
  python3 -I scripts/replay_findings.py --sample 150 --seed 1 a.json b.json ... > sample.tsv

--summary prints, per project and overall, the commits replayed, the commits
that changed a declaration, the commits with at least one finding, findings per commit (median, 90th percentile,
maximum) and findings by rule. --sample draws N findings (changed lines and
pending alike) deterministically for a seed, spread over rules first and
over projects within each rule, as TSV with an empty verdict column.
"""

import argparse
import json
import os
import random
import subprocess
import sys

def git(d, *args, check=True):
    return subprocess.run(["git", "-C", d, *args], capture_output=True, text=True, check=check).stdout


def ask(grounds, d, base, timeout):
    cmd = [grounds, "changed", "--root", d, "--base", base, "--format", "json", "--no-daemon", "--no-baseline",
           "--max", "100000", "--min-severity", "warning", "--exit-zero", "--color", "never"]
    p = subprocess.run(cmd, capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=timeout)
    if p.returncode != 0:
        raise RuntimeError(f"grounds changed failed ({p.returncode}) at base {base}: {p.stderr[-2000:]}")
    return json.loads(p.stdout)


def short(f):
    return {k: f.get(k) for k in ("rule", "severity", "path", "line", "message")}


def run(a):
    d = os.path.abspath(a.dir)
    grounds = os.path.abspath(a.grounds)
    if git(d, "status", "--porcelain", "--untracked-files=no").strip():
        sys.exit(f"{d}: uncommitted changes to tracked files; refusing to move the checkout")
    head = git(d, "rev-parse", "HEAD").strip()
    back = git(d, "symbolic-ref", "-q", "--short", "HEAD", check=False).strip() or head
    if a.from_:
        with open(a.from_) as fh:
            commits = [s["commit"] for s in json.load(fh)["steps"]]
    else:
        line = git(d, "rev-list", "--first-parent", head).split()
        commits = []
        for i in range(len(line) - 1):
            if git(d, "diff", "--name-only", line[i + 1], line[i], "--", "*.java").split():
                commits.append(line[i])
                if len(commits) == a.commits:
                    break
        commits.reverse()
    steps = []
    try:
        for c in commits:
            parent = git(d, "rev-parse", c + "^").strip()
            git(d, "checkout", "-q", "--no-overwrite-ignore", "--detach", c)
            r = ask(grounds, d, parent, a.timeout)
            steps.append({"commit": c, "parent": parent,
                          "findings": [short(f) for f in r.get("findings", [])],
                          "pending": [short(f) for f in r.get("pending", [])],
                          "breaks": len(r.get("breaks", [])),
                          "changed_apis": len(r.get("changed_apis", []))})
    finally:
        git(d, "checkout", "-q", "--no-overwrite-ignore", *(["--detach", head] if back == head else [back]))
    with open(a.out, "w") as fh:
        json.dump({"project": os.path.basename(d), "head": head, "steps": steps}, fh, indent=1)
    told = sum(1 for s in steps if s["findings"] or s["pending"])
    print(f"{os.path.basename(d)}: {len(steps)} commits, {told} with a finding, "
          f"{sum(len(s['findings']) + len(s['pending']) for s in steps)} findings")
    return 0


def load(paths):
    out = []
    for p in paths:
        with open(p) as fh:
            out.append(json.load(fh))
    return out


def everything(step):
    """What one commit tells an agent: its findings, then the places it broke elsewhere."""
    return step["findings"] + [dict(f, pending=True) for f in step["pending"]]


def pct(xs, q):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, max(0, -(-q * len(xs) // 100) - 1))] if xs else 0


def summary(paths):
    reports = load(paths)
    rows, by_rule, all_counts, all_apis = [], {}, [], 0
    for r in reports:
        counts = [len(everything(s)) for s in r["steps"]]
        apis = sum(1 for s in r["steps"] if s.get("changed_apis"))
        all_counts += counts
        all_apis += apis
        for s in r["steps"]:
            for f in everything(s):
                by_rule[(f["rule"], f["severity"])] = by_rule.get((f["rule"], f["severity"]), 0) + 1
        rows.append((r["project"], len(counts), apis, sum(1 for c in counts if c), sum(counts),
                     pct(counts, 50), pct(counts, 90), max(counts, default=0)))
    print("project\tcommits\tchanging declarations\twith a finding\tfindings\tmedian\tp90\tmax")
    for row in rows:
        print("\t".join(map(str, row)))
    print("\t".join(map(str, ("ALL", len(all_counts), all_apis, sum(1 for c in all_counts if c), sum(all_counts),
                              pct(all_counts, 50), pct(all_counts, 90), max(all_counts, default=0)))))
    print("\nrule\tseverity\tfindings")
    for (rule, sev), n in sorted(by_rule.items(), key=lambda kv: (-kv[1], kv[0])):
        print(f"{rule}\t{sev}\t{n}")
    return 0


def sample(paths, n, seed):
    rng = random.Random(seed)
    by_rule = {}
    for r in load(paths):
        for s in r["steps"]:
            for f in everything(s):
                by_rule.setdefault(f["rule"], {}).setdefault(r["project"], []).append((s["commit"], f))
    queues = {}
    for rule in sorted(by_rule):
        projects = sorted(by_rule[rule])
        rng.shuffle(projects)
        for p in projects:
            rng.shuffle(by_rule[rule][p])
        # round robin over the projects of the rule
        q, lists = [], [list(by_rule[rule][p]) for p in projects]
        while any(lists):
            for p, l in zip(projects, lists):
                if l:
                    q.append((p, *l.pop()))
        queues[rule] = q
    picked, rules = [], sorted(queues)
    rng.shuffle(rules)
    while len(picked) < n and any(queues[r] for r in rules):
        for r in rules:
            if queues[r] and len(picked) < n:
                picked.append(queues[r].pop(0))
    print("rule\tseverity\tproject\tcommit\tpath\tline\tpending\tmessage\tverdict\treason")
    for p, c, f in sorted(picked, key=lambda x: (x[2]["rule"], x[0], x[2]["path"], x[2]["line"])):
        print("\t".join(map(str, (f["rule"], f["severity"], p, c, f["path"], f["line"], "yes" if f.get("pending") else "no",
                                  f["message"].replace("\t", " ").replace("\n", " "), "", ""))))
    return 0


def main():
    if len(sys.argv) > 2 and sys.argv[1] == "--summary":
        return summary(sys.argv[2:])
    if len(sys.argv) > 1 and sys.argv[1] == "--sample":
        ap = argparse.ArgumentParser()
        ap.add_argument("--sample", type=int, required=True)
        ap.add_argument("--seed", type=int, default=1)
        ap.add_argument("reports", nargs="+")
        a = ap.parse_args()
        return sample(a.reports, a.sample, a.seed)
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--from", dest="from_", help="a replay.py --out report: replay its commits")
    ap.add_argument("--commits", type=int, default=15, help="without --from: commits that change a Java file")
    ap.add_argument("--grounds", default=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                                                      "target", "release", "grounds"))
    ap.add_argument("--timeout", type=float, default=900.0)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    if a.commits < 1:
        ap.error("--commits must be at least 1")
    return run(a)


if __name__ == "__main__":
    sys.exit(main())
