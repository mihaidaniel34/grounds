#!/usr/bin/env python3
"""Hold-out precision measurement: grounds on projects the rules were never tuned on.

Every false positive found on the corpus (corpus/projects.tsv) became a
fixture and was fixed, so the corpus no longer says much about precision on
other code. corpus/holdout.tsv pins other projects; this script runs grounds
on them and draws a sample of findings to judge by hand. It measures only:
nothing found here should be turned into a fixture, or the hold-out stops
being one.

  run       fetch the pinned commits (shallow) into DIR, write a grounds.toml
            with scan_caches = false where there is none (JDK only: no build
            tool runs, so library calls stay unresolved and the rules that
            need them stay silent), run `grounds check --format json` and
            write OUT/<project>.json (findings), OUT/<project>.counts.tsv
            (rule, count) and OUT/summary.tsv (files, lines, findings, time).
  sample    draw up to N findings per rule across all projects, spread over
            projects, deterministically for a seed; every finding of the
            rules named by --all (default: the compile-break and wiring
            rules); write OUT/sample.tsv and OUT/sample.md with 12 lines of
            source context around each finding.

  python3 -I scripts/holdout.py [--dir ~/grounds-holdout] [--out target/holdout]
      [--only a,b] [--grounds target/release/grounds] [--timeout 1800]
  python3 -I scripts/holdout.py --sample 8 --seed 1 [--min-severity warning]

Without --sample it runs; with --sample it only samples the last run.
"""

import argparse
import json
import os
import random
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
HARD_RULES = ("unresolved-method,missing-jdk-import,unresolved-constructor,unresolved-field,"
              "override-matches-nothing,unimplemented-abstract-method,bean-not-found,ambiguous-bean,"
              "qualifier-not-found,bean-name-conflict,circular-constructor-dependency")
SEVERITIES = {"info": 0, "warning": 1, "error": 2}
CONTEXT = 12


def projects(path, only):
    out = []
    with open(path) as fh:
        for line in fh:
            if not line.strip() or line.startswith("#"):
                continue
            name, url, commit = line.rstrip("\n").split("\t")[:3]
            if not only or name in only:
                out.append((name, url, commit))
    return out


def git(d, *args):
    return subprocess.run(["git", "-C", d, *args], capture_output=True, text=True, check=True).stdout


def fetch(d, url, commit):
    if os.path.isdir(os.path.join(d, ".git")) and git(d, "rev-parse", "HEAD").strip() == commit:
        return
    for attempt in range(4):
        try:
            if not os.path.isdir(os.path.join(d, ".git")):
                subprocess.run(["git", "init", "-q", d], check=True)
            git(d, "fetch", "-q", "--depth", "1", url, commit)
            git(d, "checkout", "-q", "--detach", "FETCH_HEAD")
            return
        except subprocess.CalledProcessError as e:
            print(f"  fetch failed ({e.stderr.strip()[:200]}), retrying", file=sys.stderr)
            time.sleep(2 ** (attempt + 1))
    raise SystemExit(f"{d}: cannot fetch {commit}")


def java_size(d):
    files = lines = 0
    for root, dirs, names in os.walk(d):
        dirs[:] = [x for x in dirs if x not in (".git", ".grounds")]
        for n in names:
            if n.endswith(".java"):
                files += 1
                with open(os.path.join(root, n), "rb") as fh:
                    lines += fh.read().count(b"\n")
    return files, lines


def build(grounds):
    if not os.path.exists(grounds):
        subprocess.run(["cargo", "build", "--release", "--quiet", "--manifest-path",
                        os.path.join(REPO, "Cargo.toml"), "-p", "grounds-cli"], check=True)


def severities(grounds):
    out = {}
    for line in subprocess.run([grounds, "rules", "--color", "never"], capture_output=True, text=True).stdout.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[1] in SEVERITIES:
            out[parts[0]] = parts[1]
    return out


def run(a):
    build(a.grounds)
    os.makedirs(a.dir, exist_ok=True)
    os.makedirs(a.out, exist_ok=True)
    summary = []
    for name, url, commit in projects(a.projects, a.only):
        d = os.path.join(a.dir, name)
        print(f"{name}: fetch {commit[:12]}", flush=True)
        fetch(d, url, commit)
        toml = os.path.join(d, "grounds.toml")
        if not os.path.exists(toml):
            with open(toml, "w") as fh:
                fh.write("[classpath]\nscan_caches = false\n")
        files, lines = java_size(d)
        start = time.monotonic()
        status = "ok"
        try:
            p = subprocess.run([a.grounds, "check", "--root", d, "--format", "json"], capture_output=True,
                               text=True, timeout=a.timeout, stdin=subprocess.DEVNULL)
            findings = json.loads(p.stdout or "[]")
            if p.returncode not in (0, 1):
                status = f"exit {p.returncode}"
            if p.stderr.strip():
                with open(os.path.join(a.out, f"{name}.stderr"), "w") as fh:
                    fh.write(p.stderr)
        except subprocess.TimeoutExpired:
            findings, status = [], f"timeout after {a.timeout}s"
        except json.JSONDecodeError:
            findings, status = [], f"bad json (exit {p.returncode})"
        secs = time.monotonic() - start
        with open(os.path.join(a.out, f"{name}.json"), "w") as fh:
            json.dump(findings, fh, indent=1)
        counts = {}
        for f in findings:
            counts[f["rule"]] = counts.get(f["rule"], 0) + 1
        with open(os.path.join(a.out, f"{name}.counts.tsv"), "w") as fh:
            for rule, n in sorted(counts.items(), key=lambda x: (-x[1], x[0])):
                fh.write(f"{rule}\t{n}\n")
        summary.append((name, files, lines, len(findings), f"{secs:.2f}", status))
        print(f"  {files} files, {lines} lines, {len(findings)} findings, {secs:.2f} s, {status}", flush=True)
    with open(os.path.join(a.out, "summary.tsv"), "w") as fh:
        fh.write("project\tjava_files\tlines\tfindings\tseconds\tstatus\n")
        for row in summary:
            fh.write("\t".join(map(str, row)) + "\n")
    return 0 if all(r[5] == "ok" for r in summary) else 1


def context(d, path, line):
    try:
        with open(os.path.join(d, path), encoding="utf-8", errors="replace") as fh:
            src = fh.read().split("\n")
    except OSError:
        return "(source not readable)"
    lo, hi = max(1, line - CONTEXT), min(len(src), line + CONTEXT)
    return "\n".join(f"{'>' if i == line else ' '}{i:5d}  {src[i - 1]}" for i in range(lo, hi + 1))


def sample(a):
    build(a.grounds)
    sev = severities(a.grounds)
    take_all = set(r for r in a.all.split(",") if r)
    by_rule = {}
    names = [p[0] for p in projects(a.projects, a.only)]
    for name in names:
        path = os.path.join(a.out, f"{name}.json")
        if not os.path.exists(path):
            continue
        with open(path) as fh:
            for f in json.load(fh):
                by_rule.setdefault(f["rule"], {}).setdefault(name, []).append(f)
    picked = []
    for rule in sorted(by_rule):
        if rule not in take_all and SEVERITIES.get(sev.get(rule, "info"), 0) < SEVERITIES[a.min_severity]:
            continue
        per = by_rule[rule]
        if rule in take_all:
            chosen = [(p, f) for p in sorted(per) for f in per[p]]
        else:
            # round-robin over projects in a seeded order, findings shuffled within each
            rng = random.Random(f"{a.seed}:{rule}")
            order = sorted(per)
            rng.shuffle(order)
            pools = {p: sorted(per[p], key=lambda f: (f["path"], f["line"])) for p in order}
            for p in order:
                rng.shuffle(pools[p])
            chosen = []
            while len(chosen) < a.sample and any(pools.values()):
                for p in order:
                    if pools[p] and len(chosen) < a.sample:
                        chosen.append((p, pools[p].pop()))
        total = sum(len(v) for v in per.values())
        for p, f in sorted(chosen, key=lambda x: (x[0], x[1]["path"], x[1]["line"])):
            picked.append((rule, sev.get(rule, "?"), total, p, f))
    with open(os.path.join(a.out, "sample.tsv"), "w") as fh:
        fh.write("rule\tseverity\trule_total\tproject\tpath\tline\tmessage\n")
        for rule, s, total, p, f in picked:
            msg = f["message"].replace("\t", " ").replace("\n", " ")
            fh.write(f"{rule}\t{s}\t{total}\t{p}\t{f['path']}\t{f['line']}\t{msg}\n")
    with open(os.path.join(a.out, "sample.md"), "w") as fh:
        for rule, s, total, p, f in picked:
            fh.write(f"## {rule} ({s}) {p} {f['path']}:{f['line']}\n\n{f['message']}\n\n```java\n")
            fh.write(context(os.path.join(a.dir, p), f["path"], f["line"]) + "\n```\n\n")
    print(f"{len(picked)} findings from {len({r for r, *_ in picked})} rules -> {a.out}/sample.tsv, sample.md")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--projects", default=os.path.join(REPO, "corpus", "holdout.tsv"))
    ap.add_argument("--dir", default=os.path.expanduser("~/grounds-holdout"))
    ap.add_argument("--out", default=os.path.join(REPO, "target", "holdout"))
    ap.add_argument("--grounds", default=os.path.join(REPO, "target", "release", "grounds"))
    ap.add_argument("--only", type=lambda s: set(s.split(",")), default=None)
    ap.add_argument("--timeout", type=int, default=1800)
    ap.add_argument("--sample", type=int, default=None, help="up to N findings per rule")
    ap.add_argument("--seed", default="1")
    ap.add_argument("--min-severity", default="warning", choices=sorted(SEVERITIES))
    ap.add_argument("--all", default=HARD_RULES, help="rules whose findings are all taken (comma separated)")
    a = ap.parse_args()
    a.grounds = os.path.abspath(a.grounds)
    return sample(a) if a.sample is not None else run(a)


if __name__ == "__main__":
    sys.exit(main())
