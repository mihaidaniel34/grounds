#!/usr/bin/env python3
"""Differential fuzzer for overload resolution and generic inference: grounds vs javac.

Each case is a class with 2-4 static `pick` overloads (generic or not) and a
call `var r = pick(ARGS); r.zzProbe();`. javac's diagnostic for the probe names
the call's exact type (`var`: no target type); an error on the `pick` line means
javac rejects the call. grounds' answer comes from the `call_types` example of
grounds-semantic. grounds may say unknown (`?unknown`, possibly nested) at any
time; it must not be certain of a type javac disagrees with, on valid Java.
And on a case javac accepts, the rules that claim code does not compile
(unresolved-method, override-matches-nothing, ...) must report nothing.

Deterministic: a seed fixes the cases.

  python3 -I scripts/fuzz_overloads.py --seed 1 --cases 2000 [--out DIR]
      [--baseline scripts/fuzz_overloads.baseline.json] [--update-baseline]

Needs `javac` (JDK 21+) on PATH and builds the example with cargo. Exit status
1 if any (seed, case) is wrong that the baseline does not list (a regression).
"""
import argparse
import json
import os
import random
import re
import shutil
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

PRELUDE = [
    "import java.util.*;",
    "class A implements Comparable<A> { public int compareTo(A o) { return 0; } }",
    "class B extends A {}",
    "class Bad implements Comparable<String> { public int compareTo(String o) { return 0; } }",
    "class Raw<X> extends java.util.ArrayList {}",
    "interface Supplier<T> { T get(); }",
]

# (type-parameter list, weight)
TP_CHOICES = [
    ([], 4),
    (["T"], 4),
    (["T extends Number"], 2),
    (["T extends Comparable<? super T>"], 2),
    (["T extends CharSequence"], 1),
    (["K", "V"], 2),
    (["T", "U extends T"], 2),
    (["T extends Object & Comparable<T>"], 1),
    (["T extends Comparable<T>"], 1),
    (["T extends A"], 1),
]

CONC_TYPES = [
    "String", "Object", "CharSequence", "Integer", "Number", "int", "long", "double",
    "A", "B", "Bad", "Raw<String>", "Raw<Integer>", "Raw", "String[]", "int[]", "Object[]",
    "Integer[]", "List<String>", "List<?>", "List<? extends Number>", "List<? super Integer>",
    "Map<String,Integer>", "Supplier<String>", "Class<String>", "Comparable<A>", "Comparable<?>",
    "Collection<Integer>", "Iterable<String>", "StringBuilder", "ArrayList<Integer>",
    "List<String>[]", "Map<String,Object>", "Comparable<String>",
]
CONC_RETS = [
    "Integer", "String", "Long", "Double", "Character", "Boolean", "Byte", "Short", "Float",
    "Object", "Number", "CharSequence", "List<String>", "Map<String,Integer>", "int", "long",
    "double", "String[]", "Comparable<String>", "Iterable<String>", "A", "Bad", "Raw<String>",
    "Supplier<String>", "ArrayList<Integer>", "Set<String>",
]

# local name, declared type, initializer
LOCALS = [
    ("s", "String", "null"), ("o", "Object", "null"), ("i", "Integer", "null"),
    ("n", "int", "0"), ("l", "long", "0L"), ("d", "double", "0.0"),
    ("cs", "CharSequence", "null"), ("sb", "StringBuilder", "null"),
    ("ls", "List<String>", "null"), ("al", "ArrayList<Integer>", "null"),
    ("lw", "List<?>", "null"), ("le", "List<? extends Number>", "null"),
    ("lsu", "List<? super Integer>", "null"), ("sa", "String[]", "null"),
    ("ia", "int[]", "null"), ("m", "Map<String,Integer>", "null"),
    ("a", "A", "null"), ("b", "B", "null"), ("bad", "Bad", "null"),
    ("raw", "Raw<String>", "null"), ("rw", "Raw", "null"),
    ("sup", "Supplier<String>", "null"), ("cl", "Class<String>", "null"),
    ("ca", "Comparable<A>", "null"), ("ce", "Collection<? extends Number>", "null"),
    ("it", "Iterable<String>", "null"), ("oa", "Object[]", "null"),
    ("lsa", "List<String>[]", "null"), ("cmp", "Comparable<?>", "null"),
]
LOCAL_NAMES = [x[0] for x in LOCALS]
INLINE_ARGS = [
    "null", '"lit"', "1", "1L", "() -> 1", '() -> "s"', "Object::toString", "String.class",
    "Integer.class", "new Object()", "java.util.Collections.emptyList()", "java.util.List.of(1, 2)",
    "new java.util.ArrayList<>()",
]
RET_BODY = {"int": "return 0;", "long": "return 0L;", "double": "return 0.0;"}


# ---------------------------------------------------------------- type helpers

def tvar_names(tps):
    return [tp.split(" extends ")[0] for tp in tps]


def erase(t, m):
    if t.endswith("..."):
        return erase(t[:-3], m) + "[]"
    if t.endswith("[]"):
        return erase(t[:-2], m) + "[]"
    s = t
    while True:
        s2 = re.sub(r"<[^<>]*>", "", s)
        if s2 == s:
            break
        s = s2
    s = s.strip()
    return m.get(s, s)


def tvmap(tps):
    m = {}
    for tp in tps:
        if " extends " in tp:
            name, bound = tp.split(" extends ", 1)
            m[name] = erase(bound.split(" & ")[0].strip(), m)
        else:
            m[tp] = "Object"
    return m


def pools(tv):
    gen = []
    if tv:
        x = tv[0]
        gen += [x, x + "[]", f"List<{x}>", f"List<? extends {x}>", f"List<? super {x}>",
                f"Collection<? extends {x}>", f"Iterable<{x}>", f"Comparable<{x}>",
                f"Comparable<? super {x}>", f"Supplier<{x}>", f"Class<{x}>", f"Map<String,{x}>",
                f"List<{x}>[]"]
    if len(tv) >= 2:
        x, y = tv[0], tv[1]
        gen += [f"Map<{x},{y}>", f"Map<{y},{x}>", f"List<{y}>", y, f"Supplier<{y}>",
                f"Collection<? super {y}>"]
    return gen


def make_overload(rng, tps):
    tv = tvar_names(tps)
    gen = pools(tv)
    n = rng.choice([0, 1, 1, 2, 2, 3])
    params = []
    for _ in range(n):
        if gen and rng.random() < 0.6:
            params.append(rng.choice(gen))
        else:
            params.append(rng.choice(CONC_TYPES))
    if rng.random() < 0.15:
        elems = ["String", "Object", "int", "Integer", "CharSequence", "A", "Comparable<String>"]
        if tv:
            elems.append(tv[0])
        params.append(rng.choice(elems) + "...")
    if gen and rng.random() < 0.5:
        ret = rng.choice(gen)
    else:
        ret = rng.choice(CONC_RETS)
    return {"tps": list(tps), "params": params, "ret": ret}


def make_case(rng):
    k = rng.randint(2, 4)
    tp_list = [tp for tp, _ in TP_CHOICES]
    tp_w = [w for _, w in TP_CHOICES]
    ovs = [make_overload(rng, rng.choices(tp_list, tp_w)[0]) for _ in range(k)]
    # Arguments: half the time, aim the call at a random overload: each position takes a
    # local whose declared erasure matches that overload's parameter erasure (so many calls
    # are valid Java); otherwise a random local / inline expression.
    target = rng.choice(ovs)
    m = len(target["params"]) if rng.random() < 0.5 else rng.choice([0, 1, 1, 2, 2, 3, 3])
    args = []
    for j in range(m):
        choice = None
        if rng.random() < 0.6 and j < len(target["params"]):
            want = erase(target["params"][j], tvmap(target["tps"]))
            cands = [x[0] for x in LOCALS if erase(x[1], {}) == want]
            if cands:
                choice = rng.choice(cands)
        if choice is None:
            choice = rng.choice(LOCAL_NAMES) if rng.random() < 0.8 else rng.choice(INLINE_ARGS)
        args.append(choice)
    return {"overloads": ovs, "args": args}


def sig_ok(ovs):
    """No two overloads may share an erasure (javac: name clash)."""
    seen = set()
    for ov in ovs:
        m = tvmap(ov["tps"])
        key = tuple(erase(p, m) for p in ov["params"])
        if key in seen:
            return False
        seen.add(key)
    return True


def render(case, pkg):
    lines = [f"package {pkg};"] + list(PRELUDE)
    lines.append("class C {")
    for ov in case["overloads"]:
        tp = f"<{', '.join(ov['tps'])}> " if ov["tps"] else ""
        ps = ", ".join(f"{t} p{j}" for j, t in enumerate(ov["params"]))
        body = RET_BODY.get(ov["ret"], "return null;")
        lines.append(f"  static {tp}{ov['ret']} pick({ps}) {{ {body} }}")
    lines.append("  void m() {")
    for name, decl, init in LOCALS:
        lines.append(f"    {decl} {name} = {init};")
    lines.append(f"    var r = pick({', '.join(case['args'])});")
    pick_line = len(lines)
    lines.append("    r.zzProbe();")
    probe_line = len(lines)
    lines.append("  }")
    lines.append("}")
    return "\n".join(lines) + "\n", pick_line, probe_line



# ---------------------------------------------------------------- javac (one run for all cases)

ERR_RE = re.compile(r"^(.+?\.java):(\d+):(\d+): (compiler\.err\.[\w.-]+)(?::\s(.*))?$")
LOC_RE = re.compile(r"compiler\.misc\.location\.1: kindname\.variable, r, (.*)\)$")


def javac_run(paths, out, tag):
    """One javac over `paths` (each its own package). Returns ({file name: [(line, key, msg)]},
    crashed): an internal javac error ends the run without diagnostics for later files."""
    argfile = os.path.join(out, f"javac-{tag}.args")
    with open(argfile, "w") as fh:
        fh.write("\n".join(paths) + "\n")
    classes = os.path.join(out, "classes", tag)
    os.makedirs(classes, exist_ok=True)
    # keep attributing every case even when one has an error in an earlier phase
    p = subprocess.run(["javac", "--release", "21", "-XDrawDiagnostics", "-XDshould-stop.ifError=FLOW", "-proc:none",
                        "-implicit:none", "-Xmaxerrs", "10000000", "-nowarn", "-d", classes, "@" + argfile],
                       capture_output=True, text=True)
    text = p.stdout + p.stderr
    errs = {}
    for line in text.splitlines():
        m = ERR_RE.match(line)
        if m:
            # raw diagnostics name the file without its directory; case names are unique
            errs.setdefault(os.path.basename(m.group(1)), []).append((int(m.group(2)), m.group(4), m.group(5) or ""))
    crashed = "An exception has occurred in the compiler" in text or "printing javac parameters to:" in text
    return errs, crashed


def javac_all(items, out, chunk=250):
    """items: {path: (pick_line, probe_line)} -> {path: outcome}. Chunks run in parallel; a
    chunk that crashes javac is retried one file at a time, and the crashing cases are
    marked as such (a javac bug, not a grounds one)."""
    from concurrent.futures import ThreadPoolExecutor
    paths = list(items)
    chunks = [paths[i:i + chunk] for i in range(0, len(paths), chunk)]
    errs, crashes = {}, set()
    with ThreadPoolExecutor(os.cpu_count() or 4) as ex:
        results = list(ex.map(lambda c: (c, javac_run(c[1], out, f"c{c[0]}")), enumerate(chunks)))
        retry = [p for (_, c), (e, crashed) in results if crashed for p in c]
        for _, (e, crashed) in results:
            if not crashed:
                errs.update(e)
        singles = list(ex.map(lambda p: (p, javac_run([p], out, "s" + os.path.basename(p)[:-5])), retry))
    for p, (e, crashed) in singles:
        if crashed:
            crashes.add(p)
        else:
            errs.update(e)
    res = {}
    for path, (pick_line, probe_line) in items.items():
        if path in crashes:
            res[path] = {"outcome": "javac-crash", "detail": ""}
            continue
        es = errs.get(os.path.basename(path), [])
        others = [e for e in es if e[0] not in (pick_line, probe_line)]
        picks = [e for e in es if e[0] == pick_line]
        if others:
            res[path] = {"outcome": "invalid", "detail": others[0][1] + " " + others[0][2]}
        elif picks:
            res[path] = {"outcome": "rejected", "detail": picks[0][1]}
        else:
            res[path] = {"outcome": "unexpected", "detail": ""}
            for _, key, msg in es:
                m = LOC_RE.search(msg)
                if m:
                    res[path] = {"outcome": "type", "type": m.group(1)}
                    break
                if key == "compiler.err.cant.deref":  # a primitive result: javac names the type
                    res[path] = {"outcome": "type", "type": msg.strip()}
                    break
    return res


# ---------------------------------------------------------------- grounds

def grounds_all(paths, out):
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "grounds-semantic", "--example", "call_types"],
                   cwd=REPO, check=True)
    exe = os.path.join(os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target"), "release", "examples", "call_types")
    listfile = os.path.join(out, "grounds.list")
    with open(listfile, "w") as fh:
        fh.write("\n".join(paths) + "\n")
    p = subprocess.run([exe, "pick(", "@" + listfile], capture_output=True, text=True, check=True)
    res = {}
    for line in p.stdout.splitlines():
        path, _, ty = line.partition("\t")
        res.setdefault(path, []).append(ty)
    return res


# Rules that claim the code does not compile: on a case javac accepts they must
# say nothing (but on the probe line, which calls a method that does not exist).
COMPILE_RULES = ["unresolved-method", "unresolved-constructor", "unresolved-field", "unresolved-static-import",
                 "override-matches-nothing", "unimplemented-abstract-method", "incompatible-assignment"]


def grounds_check(cases_dir):
    """Findings of the compile-break rules over all cases (one project, JDK only):
    {file name: [(line, rule, message)]}."""
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "grounds-cli"], cwd=REPO, check=True)
    exe = os.path.join(os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target"), "release", "grounds")
    # its own git root, or the ignore files of a repo above it (grounds' own
    # `target/`) hide every case
    if not os.path.exists(os.path.join(cases_dir, ".git")):
        subprocess.run(["git", "init", "-q", cases_dir], check=True)
    with open(os.path.join(cases_dir, "grounds.toml"), "w") as fh:
        fh.write("[classpath]\nscan_caches = false\nbuild_outputs = false\n")
    known = subprocess.run([exe, "rules", "--json"], capture_output=True, text=True, check=True).stdout
    rules = [r for r in COMPILE_RULES if f'"{r}"' in known]
    if "unresolved-method" not in rules:
        sys.exit(f"grounds check: the compile-break rules are missing ({rules})")
    p = subprocess.run([exe, "check", "--root", cases_dir, "--rules", ",".join(rules), "--format", "json"],
                       capture_output=True, text=True)
    if p.returncode not in (0, 1, 2) or not p.stdout.strip():
        sys.exit(f"grounds check failed ({p.returncode}): {p.stderr[-2000:]}")
    res = {}
    for f in json.loads(p.stdout or "[]"):
        res.setdefault(os.path.basename(f["path"]), []).append((f["line"], f["rule"], f["message"]))
    return res


# ---------------------------------------------------------------- main

def norm(t):
    return re.sub(r"\s+", "", t)


def judge(g, j):
    """'unknown' | 'agree' | 'wrong' (certain, valid Java, other type) | 'rejected' (certain, javac
    rejects the call) | 'other' (javac outcome not usable)."""
    if not g or len(g) != 1 or "?unknown" in g[0] or g[0].startswith("?"):
        return "unknown"
    if j["outcome"] == "type":
        return "agree" if norm(g[0]) == norm(j["type"]) else "wrong"
    if j["outcome"] == "rejected":
        return "rejected"
    return "other"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, required=True)
    ap.add_argument("--cases", type=int, default=2000)
    ap.add_argument("--out", default=None)
    ap.add_argument("--baseline", default=os.path.join(REPO, "scripts", "fuzz_overloads.baseline.json"))
    ap.add_argument("--update-baseline", action="store_true")
    a = ap.parse_args()
    out = os.path.abspath(a.out or os.path.join(REPO, "target", "fuzz-overloads", f"seed-{a.seed}"))
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(os.path.join(out, "cases"))

    rng = random.Random(a.seed)
    items, cases = {}, {}
    k = 0
    while len(items) < a.cases:
        case = make_case(rng)
        if not sig_ok(case["overloads"]):
            continue
        k += 1
        pkg = f"f{a.seed}_{k:05d}"
        src, pick_line, probe_line = render(case, pkg)
        path = os.path.join(out, "cases", pkg + ".java")
        with open(path, "w") as fh:
            fh.write(src)
        items[path] = (pick_line, probe_line)
        cases[path] = pkg

    jv = javac_all(items, out)
    gr = grounds_all(list(items), out)
    findings = grounds_check(os.path.join(out, "cases"))
    counts, bad = {}, []
    for path in items:
        verdict = judge(gr.get(path), jv[path])
        counts[verdict] = counts.get(verdict, 0) + 1
        if verdict in ("wrong", "rejected"):
            bad.append({"case": cases[path], "verdict": verdict, "grounds": gr.get(path), "javac": jv[path]})
        # valid Java: a compile-break finding is a false positive
        if jv[path]["outcome"] == "type":
            fs = [f for f in findings.get(os.path.basename(path), []) if f[0] != items[path][1]]
            if fs:
                counts["false-error"] = counts.get("false-error", 0) + 1
                bad.append({"case": cases[path], "verdict": "false-error", "grounds": [f"{r}: {m}" for _, r, m in fs], "javac": jv[path]})

    with open(os.path.join(out, "report.json"), "w") as fh:
        json.dump({"seed": a.seed, "cases": a.cases, "counts": counts, "bad": bad}, fh, indent=1)
    crashes = sorted(cases[p] for p, j in jv.items() if j["outcome"] == "javac-crash")
    print(f"seed {a.seed}: " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())) + (f"; javac crashed on {', '.join(crashes)}" if crashes else ""))

    base = {}
    if os.path.exists(a.baseline) and os.path.getsize(a.baseline) > 0:
        with open(a.baseline) as fh:
            base = json.load(fh)
    key = f"{a.seed}:{a.cases}"
    # a case is known per verdict: wrong in the baseline does not cover a new false error
    known = {("wrong", c) for c in base.get(key, {}).get("wrong", [])} | {("false-error", c) for c in base.get(key, {}).get("false_errors", [])}
    wrong = sorted(b["case"] for b in bad if b["verdict"] == "wrong")
    false_errors = sorted(b["case"] for b in bad if b["verdict"] == "false-error")
    if a.update_baseline:
        base[key] = {"wrong": wrong, "false_errors": false_errors, "rejected": sum(1 for b in bad if b["verdict"] == "rejected")}
        with open(a.baseline, "w") as fh:
            json.dump(base, fh, indent=1, sort_keys=True)
            fh.write("\n")
        print(f"baseline {key}: {len(wrong)} wrong recorded")
        return 0
    now = {("wrong", c) for c in wrong} | {("false-error", c) for c in false_errors}
    new_keys = now - known
    new = sorted({c for _, c in new_keys})
    fixed = sorted(f"{c} ({v})" for v, c in known - now)
    if fixed:
        print(f"fixed since the baseline: {len(fixed)} (run with --update-baseline)")
    for b in bad:
        if (b["verdict"], b["case"]) not in new_keys:
            continue
        if b["verdict"] == "wrong":
            print(f"WRONG {b['case']}: grounds {b['grounds'][0]}, javac {b['javac'].get('type')}  ({out}/cases/{b['case']}.java)")
        elif b["verdict"] == "false-error":
            print(f"FALSE ERROR {b['case']} (javac accepts it): {'; '.join(b['grounds'])}  ({out}/cases/{b['case']}.java)")
    return 1 if new else 0


if __name__ == "__main__":
    sys.exit(main())
