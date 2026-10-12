#!/usr/bin/env python3
"""Differential fuzzer for call typing in context: grounds vs javac.

The second generation of scripts/fuzz_overloads.py. Each case declares 2-4
`pick` overloads (static or instance, some in a superclass) and calls one of
them in a random context:

  var        var r = pick(...);
  assign     T r = pick(...);                  (the target drives inference)
  arg        sink(pick(...));                  (a parameter as target)
  lambda     Supplier<T> z = () -> pick(...);  (a lambda return)
  cond       T r = flag ? pick(...) : null;

Arguments nest: locals, literals, lambdas and method references, helper
generic calls (H.id, H.first, H.listOf, H.apply, ...) and real JDK generic
methods (Objects.requireNonNullElse, List.of, Optional.map, ...); the enclosing
method is sometimes generic itself (`<W extends Number> void m(W u, ...)`).

javac decides through `--debug=verboseResolution=all`: the overload it picked
for the judged `pick` call, and for a generic one its instantiated signature in
context (`deferred.method.inst`); an unchecked call has its result erased
(JLS 15.12.2.6). grounds' answer comes from the `call_types` example. grounds
may say unknown at any time; it must not be certain of a type javac
disagrees with, on valid Java. Counts are also broken down per feature.

Deterministic: a seed fixes the cases.

  python3 -I scripts/fuzz_calls.py --seed 1 --cases 2000 [--out DIR]
      [--baseline scripts/fuzz_calls.baseline.json] [--update-baseline]

Needs `javac` (JDK 21+) on PATH and builds the example with cargo. Exit status
1 if a (seed, case) is wrong that the baseline does not list.
"""

import argparse
import json
import os
import random
import re
import shutil
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fuzz_overloads import REPO, TP_CHOICES, erase, tvar_names, tvmap  # noqa: E402

PRELUDE = [
    "import java.util.*;",
    "import java.util.function.*;",
    "class A implements Comparable<A> { public int compareTo(A o) { return 0; } }",
    "class B extends A {}",
    "class Bad implements Comparable<String> { public int compareTo(String o) { return 0; } }",
    "class Raw<X> extends java.util.ArrayList {}",
    "class H {",
    "  static <T> T id(T x) { return x; }",
    "  static <T> T any() { return null; }",
    "  static <T> T first(List<? extends T> xs) { return null; }",
    "  static <T> T firstSuper(List<? super T> xs) { return null; }",
    "  @SafeVarargs static <T> List<T> listOf(T... xs) { return null; }",
    "  static <K, V> Map<K, V> mapOf(K k, V v) { return null; }",
    "  static <T, R> R apply(T x, Function<? super T, ? extends R> f) { return null; }",
    "  static <T> T orElse(Optional<T> o, T other) { return null; }",
    "}",
]

CONC_TYPES = [
    "String", "Object", "CharSequence", "Integer", "Number", "int", "long", "double",
    "A", "B", "Bad", "Raw<String>", "Raw", "String[]", "int[]", "Object[]", "Integer[]",
    "List<String>", "List<?>", "List<? extends Number>", "List<? super Integer>",
    "Map<String,Integer>", "Supplier<String>", "Class<String>", "Comparable<A>", "Comparable<?>",
    "Collection<Integer>", "Iterable<String>", "ArrayList<Integer>", "Optional<String>",
    "Function<String,Integer>", "Runnable", "Comparator<String>", "Predicate<Object>",
]
CONC_RETS = [
    "Integer", "String", "Long", "Object", "Number", "CharSequence", "List<String>",
    "Map<String,Integer>", "int", "long", "double", "String[]", "Comparable<String>",
    "Iterable<String>", "A", "Bad", "Raw<String>", "Supplier<String>", "Optional<Integer>",
]
LOCALS = [
    ("s", "String"), ("o", "Object"), ("i", "Integer"), ("n", "int"), ("l", "long"), ("d", "double"),
    ("cs", "CharSequence"), ("ls", "List<String>"), ("al", "ArrayList<Integer>"), ("lw", "List<?>"),
    ("le", "List<? extends Number>"), ("lsu", "List<? super Integer>"), ("sa", "String[]"),
    ("ia", "int[]"), ("m", "Map<String,Integer>"), ("a", "A"), ("b", "B"), ("bad", "Bad"),
    ("raw", "Raw<String>"), ("rw", "Raw"), ("sup", "Supplier<String>"), ("cl", "Class<String>"),
    ("ca", "Comparable<A>"), ("ce", "Collection<? extends Number>"), ("it", "Iterable<String>"),
    ("oa", "Object[]"), ("cmp", "Comparable<?>"), ("os", "Optional<String>"),
    ("fn", "Function<String,Integer>"), ("flag", "boolean"),
]
INIT = {"int": "0", "long": "0L", "double": "0.0", "boolean": "false"}
LITERALS = ["null", '"lit"', "1", "1L", "2.0", "'c'", "true", "String.class", "new Object()", "new A()", "new ArrayList<>()"]
LAMBDAS = ["x -> x", "x -> 1", 'x -> "s"', "() -> 1", '() -> "s"', "(x, y) -> x", "x -> {}", "() -> {}",
           "String::length", "Object::toString", "Integer::valueOf", "String::valueOf", "List::size", "ArrayList::new"]
TARGETS = ["Object", "String", "Integer", "Number", "CharSequence", "List<String>", "List<?>",
           "Comparable<String>", "Iterable<String>", "Collection<Integer>", "A", "int", "long", "double",
           "Optional<String>", "Map<String,Integer>"]
BOUNDS = ["Object", "Number", "CharSequence", "Comparable<W>", "A"]
RET_BODY = {"int": "return 0;", "long": "return 0L;", "double": "return 0.0;", "void": ""}


def pools(tv):
    gen = []
    if tv:
        x = tv[0]
        gen += [x, x + "[]", f"List<{x}>", f"List<? extends {x}>", f"List<? super {x}>",
                f"Collection<? extends {x}>", f"Iterable<{x}>", f"Comparable<? super {x}>",
                f"Supplier<{x}>", f"Class<{x}>", f"Map<String,{x}>", f"Optional<{x}>",
                f"Function<{x},String>", f"Function<? super {x},? extends {x}>", f"Predicate<{x}>",
                f"Comparator<{x}>", f"UnaryOperator<{x}>"]
    if len(tv) >= 2:
        x, y = tv[0], tv[1]
        gen += [f"Map<{x},{y}>", f"List<{y}>", y, f"Function<{x},{y}>", f"Function<? super {x},? extends {y}>",
                f"BiFunction<{x},{x},{y}>"]
    return gen


def make_overload(rng, tps):
    tv = tvar_names(tps)
    gen = pools(tv)
    params = [rng.choice(gen) if gen and rng.random() < 0.6 else rng.choice(CONC_TYPES)
              for _ in range(rng.choice([0, 1, 1, 2, 2, 3]))]
    if rng.random() < 0.12:
        elems = ["String", "Object", "int", "Integer", "CharSequence", "A"] + tv[:1]
        params.append(rng.choice(elems) + "...")
    ret = rng.choice(gen) if gen and rng.random() < 0.5 else rng.choice(CONC_RETS)
    return {"tps": list(tps), "params": params, "ret": ret,
            "where": rng.choice(["C", "C", "Base"]), "static": rng.random() < 0.6}


def sig_ok(ovs):
    """No two overloads may share an erasure (javac: name clash, override clash)."""
    seen = set()
    for ov in ovs:
        key = tuple(erase(p, tvmap(ov["tps"])) for p in ov["params"])
        if key in seen:
            return False
        seen.add(key)
    return True


FUNCTIONAL = {  # erasure -> lambdas / references of the right shape
    "Function": ["x -> x", "x -> 1", 'x -> "s"', "String::valueOf", "Object::toString", "x -> null"],
    "UnaryOperator": ["x -> x", "x -> null"],
    "BiFunction": ["(x, y) -> x", "(x, y) -> null"],
    "Supplier": ["() -> 1", '() -> "s"', "() -> null", "A::new", "ArrayList::new"],
    "Predicate": ["x -> true", "Objects::isNull"],
    "Comparator": ["(x, y) -> 0", "Comparator.naturalOrder()"],
    "Runnable": ["() -> {}"],
}
LISTS = {"List", "Collection", "Iterable", "ArrayList"}
CONC_VALUES = {"String": ['"lit"', "String.valueOf(1)"], "Integer": ["1", "Integer.valueOf(1)"], "Object": ["new Object()", '"lit"'],
               "Number": ["1", "2.0"], "CharSequence": ['"lit"'], "int": ["1"], "long": ["1L"], "double": ["2.0"],
               "A": ["new A()"], "B": ["new B()"], "Long": ["1L"], "Character": ["'c'"]}


def value(rng, locals_, depth, feats):
    """Any value: a local, a literal, or a generic call around values."""
    r = rng.random()
    if depth < 2 and r < 0.25:
        feats.add("nested-helper")
        v = lambda: value(rng, locals_, depth + 1, feats)
        return rng.choice([lambda: f"H.id({v()})", lambda: "H.any()", lambda: f"H.first({rng.choice(['ls', 'le', 'al', 'lw'])})",
                           lambda: f"H.firstSuper({rng.choice(['ls', 'lsu', 'lw'])})", lambda: f"H.apply({v()}, {rng.choice(FUNCTIONAL['Function'])})",
                           lambda: f"H.orElse({rng.choice(['os', 'Optional.empty()'])}, {v()})"])()
    if depth < 2 and r < 0.45:
        feats.add("jdk-generic")
        v = lambda: value(rng, locals_, depth + 1, feats)
        return rng.choice([lambda: f"Objects.requireNonNullElse({v()}, {v()})", lambda: f"Objects.requireNonNull({v()})",
                           lambda: f"Optional.ofNullable({v()}).orElse({v()})", lambda: f"Optional.of({v()}).map({rng.choice(FUNCTIONAL['Function'])}).orElse(null)",
                           lambda: f"Collections.max({rng.choice(['ls', 'al'])})", lambda: f"Map.entry({v()}, {v()}).getValue()"])()
    if r < 0.6:
        return rng.choice(sum(CONC_VALUES.values(), []))
    return rng.choice([n for n, t in locals_ if t not in ("boolean",)])


def arg_for(rng, ptype, tvars, locals_, depth, feats):
    """An argument shaped for the parameter type, mostly; sometimes anything."""
    if rng.random() < 0.1:
        return rng.choice(LAMBDAS + LITERALS + [n for n, _ in locals_])
    t = ptype.replace("...", "")
    head = re.match(r"[\w.]+", t.lstrip("? ")).group(0) if re.match(r"[\w.]+", t.lstrip("? ")) else t
    er = erase(t, {})
    same = [n for n, lt in locals_ if erase(lt, {}) == er and not t.endswith("[]")]
    if head in tvars and not t.endswith("[]"):
        return value(rng, locals_, depth, feats)
    if head in FUNCTIONAL:
        feats.add("lambda")
        own = [n for n, lt in locals_ if erase(lt, {}) == head]
        return rng.choice(FUNCTIONAL[head] + own)
    if head in LISTS and not t.endswith("[]"):
        v = lambda: value(rng, locals_, depth + 1, feats)
        opts = same + [n for n, lt in locals_ if erase(lt, {}) in ("List", "ArrayList", "Raw") ]
        if depth < 2:
            feats.add("jdk-generic")
            opts += [f"List.of({v()})", f"List.of({v()}, {v()})", "Collections.emptyList()", f"Arrays.asList({v()}, {v()})",
                     f"H.listOf({v()}, {v()})", "new ArrayList<>()", f"Collections.singletonList({v()})"]
        return rng.choice(opts)
    if head == "Optional":
        return rng.choice(["os", f"Optional.ofNullable({value(rng, locals_, depth + 1, feats)})", "Optional.empty()"])
    if head == "Map":
        return rng.choice(["m", f"H.mapOf({value(rng, locals_, depth + 1, feats)}, {value(rng, locals_, depth + 1, feats)})", "new HashMap<>()"])
    if head == "Class":
        return rng.choice(["cl", "String.class", "Integer.class", "cu" if any(n == "cu" for n, _ in locals_) else "cl"])
    if same and rng.random() < 0.6:
        return rng.choice(same)
    if er in CONC_VALUES:
        vals = CONC_VALUES[er]
        if depth < 2 and rng.random() < 0.4:
            feats.add("jdk-generic")
            return rng.choice([f"Objects.requireNonNull({rng.choice(vals)})", f"H.id({rng.choice(vals)})",
                               f"Objects.requireNonNullElse({rng.choice(same or vals)}, {rng.choice(vals)})"])
        return rng.choice(vals)
    return rng.choice(same) if same else value(rng, locals_, depth, feats)


def make_case(rng):
    k = rng.randint(2, 4)
    tp_list = [tp for tp, _ in TP_CHOICES]
    tp_w = [w for _, w in TP_CHOICES]
    feats = set()
    ovs = [make_overload(rng, rng.choices(tp_list, tp_w)[0]) for _ in range(k)]
    caller = None
    if rng.random() < 0.3:
        caller = rng.choice(BOUNDS)
        feats.add("generic-caller")
    locals_ = list(LOCALS) + ([("u", "W"), ("lu", "List<W>"), ("cu", "Class<W>")] if caller else [])
    target = rng.choice(ovs)
    params = target["params"]
    if params and params[-1].endswith("...") and rng.random() < 0.5:
        params = params[:-1] + [params[-1]] * rng.choice([0, 2])  # varargs: none or two elements
    nargs = len(params) if rng.random() < 0.85 else rng.choice([0, 1, 2, 3])
    tv = tvar_names(target["tps"]) + (["W"] if caller else [])
    args = [arg_for(rng, params[j], tv, locals_, 0, feats) if j < len(params) else value(rng, locals_, 0, feats)
            for j in range(nargs)]
    context = rng.choice(["var", "var", "assign", "assign", "arg", "lambda", "cond"])
    # a target mostly compatible with what the chosen overload returns
    ret = target["ret"]
    if any(re.search(rf"\b{v}\b", ret) for v in tvar_names(target["tps"])):
        tgt = rng.choice(TARGETS)
    else:
        tgt = rng.choices([ret if ret != "void" else "Object", "Object", rng.choice(TARGETS)], [0.6, 0.25, 0.15])[0]
    feats.add("ctx-" + context)
    if any(o["where"] == "Base" for o in ovs):
        feats.add("inherited")
    if any(not o["static"] for o in ovs):
        feats.add("instance")
    if any(p.endswith("...") for o in ovs for p in o["params"]):
        feats.add("varargs")
    return {"overloads": ovs, "args": args, "context": context, "target": tgt,
            "caller": caller, "features": sorted(feats)}


def render(case, pkg):
    lines = [f"package {pkg};"] + PRELUDE
    for cls in ("Base", "C"):
        lines.append("class Base {" if cls == "Base" else "class C extends Base {")
        for ov in case["overloads"]:
            if ov["where"] != cls:
                continue
            tp = f"<{', '.join(ov['tps'])}> " if ov["tps"] else ""
            ps = ", ".join(f"{t} p{j}" for j, t in enumerate(ov["params"]))
            mods = "static " if ov["static"] else ""
            lines.append(f"  {mods}{tp}{ov['ret']} pick({ps}) {{ {RET_BODY.get(ov['ret'], 'return null;')} }}")
        if cls == "C":
            t = case["target"]
            lines.append(f"  static void sink({t} x) {{}}")
            caller = case["caller"]
            head = f"<W extends {caller}> void m(W u, List<W> lu, Class<W> cu)" if caller else "void m()"
            lines.append(f"  {head} {{")
            for name, decl in LOCALS:
                lines.append(f"    {decl} {name} = {INIT.get(decl, 'null')};")
            call = f"pick({', '.join(case['args'])})"
            stmt = {
                "var": f"var r = {call};",
                "assign": f"{t} r = {call};",
                "arg": f"sink({call});",
                "lambda": f"Supplier<{box(t)}> z = () -> {call};",
                "cond": f"{t} r = flag ? {call} : {zero(t)};",
            }[case["context"]]
            lines.append("    " + stmt)
            judged = len(lines)
            lines.append("  }")
        lines.append("}")
    return "\n".join(lines) + "\n", judged


def box(t):
    return {"int": "Integer", "long": "Long", "double": "Double"}.get(t, t)


def zero(t):
    return INIT.get(t, "null")


# ---------------------------------------------------------------- javac

LINE_RE = re.compile(r"^(.+?\.java):(\d+):(\d+): ((?:compiler\.(?:err|warn|note))\.[\w.-]+)(?::\s(.*))?$")


def javac_run(paths, out, tag):
    argfile = os.path.join(out, f"javac-{tag}.args")
    with open(argfile, "w") as fh:
        fh.write("\n".join(paths) + "\n")
    classes = os.path.join(out, "classes", tag)
    os.makedirs(classes, exist_ok=True)
    p = subprocess.run(["javac", "--release", "21", "-XDrawDiagnostics", "--debug=verboseResolution=all",
                        "-Xlint:unchecked", "-XDshould-stop.ifError=FLOW", "-proc:none", "-implicit:none",
                        "-Xmaxerrs", "100000000", "-Xmaxwarns", "100000000", "-d", classes, "@" + argfile],
                       capture_output=True, text=True)
    text = p.stdout + p.stderr
    diags = {}
    for line in text.splitlines():
        mm = LINE_RE.match(line)
        if mm:
            diags.setdefault(os.path.basename(mm.group(1)), []).append(
                (int(mm.group(2)), int(mm.group(3)), mm.group(4), mm.group(5) or ""))
    crashed = "An exception has occurred in the compiler" in text or "printing javac parameters to:" in text or p.returncode > 2
    return diags, crashed


def javac_all(paths, out, chunk=200):
    chunks = [paths[i:i + chunk] for i in range(0, len(paths), chunk)]
    diags, retry = {}, []
    with ThreadPoolExecutor(os.cpu_count() or 4) as ex:
        for c, (d, crashed) in zip(chunks, ex.map(lambda ic: javac_run(ic[1], out, f"c{ic[0]}"), enumerate(chunks))):
            if crashed:
                retry += c
            else:
                diags.update(d)
        crashes = set()
        for p, (d, crashed) in zip(retry, ex.map(lambda p: javac_run([p], out, "s" + os.path.basename(p)[:-5]), retry)):
            if crashed:
                crashes.add(os.path.basename(p))
            else:
                diags.update(d)
    return diags, crashes


def split_top(s):
    """Split on commas outside <>, () and []."""
    out, depth, cur = [], 0, ""
    for ch in s:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    out.append(cur.strip())
    return out


def ret_of_inst(inst):
    """`(params)RET` -> RET."""
    depth = 0
    for i, ch in enumerate(inst):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return inst[i + 1:].strip()
    return None


def simple(t):
    """Drop package qualifiers and spaces; `? extends Object` is `?`."""
    t = re.sub(r"\b(?:[a-z_]\w*\.)+(?=[A-Z])", "", t)  # before dropping spaces: `? super java.lang.X`
    t = re.sub(r"\s+", "", t)
    t = re.sub(r"\?extendsObject(?=[,>])", "?", t)
    return t


def erasure(t):
    while True:
        t2 = re.sub(r"<[^<>]*>", "", t)
        if t2 == t:
            return t
        t = t2


def javac_verdict(case, diags, judged):
    """{'outcome': 'type', 'type': T} | {'outcome': 'rejected'} | {'outcome': 'invalid'|'other'|'skip'}."""
    errs = [d for d in diags if d[2].startswith("compiler.err")]
    if any(d[0] == judged for d in errs):
        return {"outcome": "rejected"}
    if errs:
        return {"outcome": "invalid", "detail": errs[0][2]}
    on_line = [d for d in diags if d[0] == judged]
    found = [d for d in on_line if d[2] == "compiler.note.verbose.resolve.multi" and d[3].startswith("pick,")]
    deferred = [d for d in on_line if d[2] == "compiler.note.deferred.method.inst" and re.match(r"<[^>]*(?:<[^>]*>[^>]*)*>pick\(", d[3])]
    unchecked = any(d[2] == "compiler.warn.unchecked.meth.invocation.applied" and "pick" in d[3] for d in on_line)
    if deferred:
        parts = split_top(deferred[-1][3])
        ret = ret_of_inst(parts[1]) if len(parts) > 1 else None
    elif found:
        # `pick, C, IDX, PHASE, ...`: IDX is the candidate javac chose among those listed
        note = found[-1][3]
        idx = split_top(note)[2] if len(split_top(note)) > 2 else "-1"
        mm = re.search(r"\(compiler\.misc\.applicable\.method\.found(?:\.1)?: " + re.escape(idx) + r", pick\((.*?)\), ", note)
        if not mm:
            return {"outcome": "other", "why": "selected candidate not found in note"}
        want = [simple(p) for p in split_top(mm.group(1))] if mm.group(1) else []
        ret = None
        for ov in case["overloads"]:
            # (a generic overload whose variables no argument mentions gets no
            # instantiation note either)
            if [simple(p.replace("...", "[]")) for p in ov["params"]] == [w.replace("...", "[]") for w in want]:
                ret = ov["ret"]
        if ret is None:
            return {"outcome": "other", "why": "selected non-generic overload not matched: " + mm.group(1)}
    else:
        return {"outcome": "other", "why": "no resolution note for pick on the line"}
    if ret is None:
        return {"outcome": "other"}
    if unchecked:
        ret = erasure(ret)
    if any(x in ret for x in ("captureof", "captured", "intersection", "compiler.misc", "&")):
        return {"outcome": "skip"}
    # a type variable of `pick` left in javac's answer (an unconstrained
    # variable with a recursive bound): only the erasure is comparable
    tvs = {v for ov in case["overloads"] for v in tvar_names(ov["tps"])}
    if any(re.search(rf"\b{v}\b", ret) for v in tvs):
        if erasure(ret).strip() in tvs:  # the bare variable: its bound is all javac knows
            return {"outcome": "skip"}
        return {"outcome": "erasure", "type": erasure(ret)}
    return {"outcome": "type", "type": ret}


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
        path, _, t = line.partition("\t")
        res.setdefault(path, []).append(t)
    return res


def judge(g, j):
    if not g or len(g) != 1 or "?unknown" in g[0] or g[0].startswith("?"):
        return "unknown"
    if j["outcome"] == "type":
        return "agree" if simple(g[0]) == simple(j["type"]) else "wrong"
    if j["outcome"] == "erasure":
        return "agree" if simple(erasure(g[0])) == simple(j["type"]) else "wrong"
    if j["outcome"] == "rejected":
        return "certain-on-rejected"
    return j["outcome"]


# ---------------------------------------------------------------- main

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--cases", type=int, default=2000)
    ap.add_argument("--out")
    ap.add_argument("--baseline", default=os.path.join(REPO, "scripts", "fuzz_calls.baseline.json"))
    ap.add_argument("--update-baseline", action="store_true")
    a = ap.parse_args()
    out = os.path.abspath(a.out or os.path.join(REPO, "target", "fuzz-calls", f"seed-{a.seed}"))
    shutil.rmtree(out, ignore_errors=True)
    cases_dir = os.path.join(out, "cases")
    os.makedirs(cases_dir)
    cases, judged = {}, {}
    i = 0
    while len(cases) < a.cases:
        i += 1
        rng = random.Random(f"calls:{a.seed}:{i}")
        case = make_case(rng)
        if not sig_ok(case["overloads"]):
            continue
        name = f"g{a.seed}_{i:05d}"
        src, line = render(case, name)
        path = os.path.join(cases_dir, name + ".java")
        with open(path, "w") as fh:
            fh.write(src)
        cases[path], judged[path] = case, line
    paths = list(cases)
    diags, crashes = javac_all(paths, out)
    gr = grounds_all(paths, out)
    counts, by_feature, bad, others = {}, {}, [], []
    for path in paths:
        f = os.path.basename(path)
        jv = {"outcome": "javac-crash"} if f in crashes else javac_verdict(cases[path], diags.get(f, []), judged[path])
        verdict = judge(gr.get(path), jv)
        if jv["outcome"] in ("rejected", "invalid") and verdict != "certain-on-rejected":
            verdict = "javac-" + jv["outcome"]
        counts[verdict] = counts.get(verdict, 0) + 1
        for feat in cases[path]["features"]:
            by_feature.setdefault(feat, {}).setdefault(verdict, 0)
            by_feature[feat][verdict] += 1
        if jv["outcome"] == "other" and len(others) < 30:
            others.append({"case": f[:-5], "why": jv.get("why", "")})
        if verdict == "wrong":
            bad.append({"case": f[:-5], "grounds": gr.get(path), "javac": jv, "features": cases[path]["features"]})
    key = f"{a.seed}:{a.cases}"
    try:
        with open(a.baseline) as fh:
            base = json.load(fh)
    except (FileNotFoundError, json.JSONDecodeError):
        base = {}
    known = set(base.get(key, {}).get("wrong", []))
    wrong = sorted(b["case"] for b in bad)
    new = [c for c in wrong if c not in known]
    fixed = sorted(c for c in known if c not in wrong)
    with open(os.path.join(out, "report.json"), "w") as fh:
        json.dump({"seed": a.seed, "counts": counts, "by_feature": by_feature, "wrong": bad, "other": others}, fh, indent=1)
    if a.update_baseline:
        base[key] = {"wrong": wrong}
        with open(a.baseline, "w") as fh:
            json.dump(base, fh, indent=1, sort_keys=True)
            fh.write("\n")
    print(f"seed {a.seed}: " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())))
    for feat in sorted(by_feature):
        c = by_feature[feat]
        valid = sum(v for k, v in c.items() if k in ("agree", "unknown", "wrong"))
        print(f"  {feat:16} valid {valid:5}  agree {c.get('agree', 0):5}  unknown {c.get('unknown', 0):5}  wrong {c.get('wrong', 0):4}")
    for b in bad:
        if b["case"] in new:
            print(f"WRONG {b['case']}: grounds {b['grounds'][0]}, javac {b['javac'].get('type')}  ({cases_dir}/{b['case']}.java)")
    if fixed:
        print(f"no longer wrong (update the baseline): {', '.join(fixed)}")
    return 1 if new and not a.update_baseline else 0


if __name__ == "__main__":
    sys.exit(main())
