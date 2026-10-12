#!/usr/bin/env python3
"""CI mode under scripted edits: `grounds changed --base` on a real project.

Each mutation is one commit on top of the project's checkout:

  safe      edits that cannot break anything: a new private or public method
            with a fresh name, a comment line that shifts the whole file, a new
            class in an existing package. `changed --base HEAD~1` must report
            nothing: no broken places in other files, and no finding on the
            changed lines but on the inserted code itself (an unused private
            method may be reported there; a compile-break rule may not).
  breaking  a public method that other files call, renamed. Reporting nothing
            is allowed (unknown means silent); how often a break is reported
            is a statistic, not a failure.

Deterministic: the seed and the project's files fix the mutations. The
checkout is reset after each mutation; uncommitted changes are refused.

  python3 -I scripts/ci_mutations.py DIR [--seed 1] [--safe 12] [--breaking 6]
      [--grounds target/release/grounds] [--out report.json]

Exit status 1 if a safe edit got a report.
"""

import argparse
import json
import os
import random
import re
import subprocess
import sys

COMPILE_RULES = {"unresolved-method", "unresolved-constructor", "unresolved-field", "unresolved-static-import",
                 "override-matches-nothing", "unimplemented-abstract-method", "incompatible-assignment",
                 "removed-class-still-used", "breaks-other-files"}
CLASS_RE = re.compile(r"^(?:public\s+)?(?:(?:final|abstract)\s+)*class\s+\w+[^;{]*\{\s*$")
PACKAGE_RE = re.compile(r"^package\s+([\w.]+)\s*;")
METHOD_RE = re.compile(r"^\s+public\s+(?:(?:static|final|synchronized)\s+)*[\w.<>\[\], ?]+\s+(\w+)\s*\([^)]*\)\s*(?:throws [\w., ]+)?\s*\{\s*$")


def git(d, *args, check=True):
    return subprocess.run(["git", "-C", d, *args], capture_output=True, text=True, check=check).stdout


def sources(d):
    files = git(d, "ls-files", "--", "*.java").splitlines()
    return sorted(f for f in files if "/src/main/java/" in "/" + f and "/generated" not in f)


def read(d, f):
    with open(os.path.join(d, f), encoding="utf-8", errors="replace") as fh:
        return fh.read().split("\n")


def write(d, f, lines):
    with open(os.path.join(d, f), "w", encoding="utf-8") as fh:
        fh.write("\n".join(lines))


def insert_after_class(lines, code):
    """Insert `code` after the opening line of the first top-level class; the
    inserted line number (1-based) or None."""
    for i, line in enumerate(lines):
        if CLASS_RE.match(line):
            lines.insert(i + 1, code)
            return i + 2
    return None


def safe_mutation(rng, d, files, k):
    """(description, {file: inserted lines}) after editing the working tree, or None."""
    kind = rng.choice(["private-method", "public-method", "comment-shift", "new-class"])
    f = rng.choice(files)
    lines = read(d, f)
    if kind == "private-method":
        ln = insert_after_class(lines, f"    private int zzMut{k}() {{ return {k}; }}")
    elif kind == "public-method":
        ln = insert_after_class(lines, f"    public String zzMut{k}(int zzArg) {{ return String.valueOf(zzArg); }}")
    elif kind == "comment-shift":
        ln = next((i + 2 for i, line in enumerate(lines) if PACKAGE_RE.match(line)), None)
        if ln:
            lines.insert(ln - 1, f"// zz mutation {k}")
    else:
        pkg = next((m.group(1) for line in lines for m in [PACKAGE_RE.match(line)] if m), None)
        if not pkg:
            return None
        new = os.path.join(os.path.dirname(f), f"ZzMut{k}.java")
        write(d, new, [f"package {pkg};", "", f"class ZzMut{k} {{", "    int value() { return 1; }", "}", ""])
        return f"{kind} {new}", {new: set(range(1, 7))}
    if not ln:
        return None
    write(d, f, lines)
    return f"{kind} {f}:{ln}", {f: {ln}}


def breaking_mutation(rng, d, files, k, calls):
    """Rename a public method some other file calls; (description, file) or None."""
    f = rng.choice(files)
    lines = read(d, f)
    cands = [(i, m.group(1)) for i, line in enumerate(lines) for m in [METHOD_RE.match(line)] if m
             and m.group(1) not in ("main", "toString", "equals", "hashCode", "run", "close", "get", "set")
             and any(g != f for g in calls.get(m.group(1), ()))]
    if not cands:
        return None
    i, name = rng.choice(cands)
    if i > 0 and lines[i - 1].strip().startswith("@Override"):
        return None  # renaming an override breaks it locally; we want callers
    lines[i] = lines[i].replace(f" {name}(", f" {name}ZzRenamed(", 1)
    write(d, f, lines)
    return f"rename {f}:{i + 1} {name}", f


def call_index(d, files):
    """method name -> files with `.name(` in them (main sources)."""
    idx = {}
    for f in files:
        for name in set(re.findall(r"\.(\w+)\(", "\n".join(read(d, f)))):
            idx.setdefault(name, set()).add(f)
    return idx


def changed(grounds, d):
    p = subprocess.run([grounds, "changed", "--root", d, "--base", "HEAD~1", "--format", "json", "--no-daemon",
                        "--exit-zero", "--max", "100000", "--min-severity", "info", "--color", "never"],
                       capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(f"grounds changed failed ({p.returncode}): {p.stderr[-2000:]}")
    return json.loads(p.stdout)


def commit(d, msg):
    git(d, "add", "-A", "--", "*.java")
    git(d, "-c", "user.name=mutation", "-c", "user.email=mutation@localhost", "-c", "commit.gpgsign=false",
        "commit", "-q", "--no-verify", "-m", msg)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dir")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--safe", type=int, default=12)
    ap.add_argument("--breaking", type=int, default=6)
    ap.add_argument("--grounds", default=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                                                      "target", "release", "grounds"))
    ap.add_argument("--out")
    a = ap.parse_args()
    d = os.path.abspath(a.dir)
    grounds = os.path.abspath(a.grounds)
    if git(d, "status", "--porcelain", "--untracked-files=no").strip():
        sys.exit(f"{d}: uncommitted changes to tracked files; refusing to mutate")
    files = sources(d)
    if not files:
        sys.exit(f"{d}: no main Java sources")
    calls = call_index(d, files)
    rng = random.Random(f"{a.seed}:{os.path.basename(d)}")
    head = git(d, "rev-parse", "HEAD").strip()
    results, false_reports = [], 0
    stats = {"safe": 0, "safe-reported": 0, "breaking": 0, "breaking-reported": 0}
    plan = ["safe"] * a.safe + ["breaking"] * a.breaking
    k = 0
    try:
        for kind in plan:
            for _attempt in range(20):
                k += 1
                m = safe_mutation(rng, d, files, k) if kind == "safe" else breaking_mutation(rng, d, files, k, calls)
                if m:
                    break
                git(d, "checkout", "-q", "--", ".")
            else:
                continue
            desc = m[0]
            commit(d, desc)
            try:
                r = changed(grounds, d)
            finally:
                git(d, "reset", "-q", "--hard", head)
                git(d, "clean", "-q", "-f", "--", "*ZzMut*.java")
            pending = r.get("pending", [])
            findings = r.get("findings", [])
            stats[kind] += 1
            if kind == "safe":
                inserted = m[1]
                bad = pending + [f for f in findings if f["rule"] in COMPILE_RULES
                                 or f["line"] not in inserted.get(f["path"], ())]
                if bad:
                    stats["safe-reported"] += 1
                    false_reports += 1
                    print(f"FALSE REPORT after safe edit ({desc}):")
                    for f in bad[:10]:
                        print(f"    {f['path']}:{f['line']} {f['rule']}: {f['message']}")
                results.append({"mutation": desc, "kind": kind, "reported": [f"{f['path']}:{f['line']} {f['rule']}" for f in bad]})
            else:
                hit = bool(pending) or any(f["rule"] in COMPILE_RULES for f in findings)
                stats["breaking-reported"] += hit
                results.append({"mutation": desc, "kind": kind, "reported": hit})
    finally:
        git(d, "reset", "-q", "--hard", head)
    print(f"{os.path.basename(d)}: safe {stats['safe']} ({stats['safe-reported']} reported), "
          f"breaking {stats['breaking']} ({stats['breaking-reported']} reported)")
    if a.out:
        with open(a.out, "w") as fh:
            json.dump({"project": os.path.basename(d), "stats": stats, "mutations": results}, fh, indent=1)
    return 1 if false_reports else 0


if __name__ == "__main__":
    sys.exit(main())
