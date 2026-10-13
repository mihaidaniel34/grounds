#!/usr/bin/env python3
"""Differential fuzzer for member access: the "won't compile" rules vs javac.

Each case is one file: an interface I (abstract, default, static and private
methods, constants), sometimes a generic G<T>, an abstract class A, a class
B extends A with nested, inner, anonymous, local and lambda bodies, and a class
C using them. Members get random names (shared between methods and fields),
access, static-ness and types; 1-3 sites call, read or reference a member
through some receiver (`super.m()`, `I.super.m()`, `B.this.f`, `B::m`, a
static import...). B may override with or without @Override and may leave
abstract methods to its supertypes.

javac decides. On a file it accepts, the rules that claim code does not compile
(unresolved-method, override-matches-nothing, unimplemented-abstract-method,
...) must report nothing: any finding there is a false positive. Files javac
rejects only feed the "caught" statistic (an error line grounds also flags).

Deterministic: a seed fixes the cases.

  python3 -I scripts/fuzz_access.py --seed 1 --cases 2000 [--out DIR]
      [--baseline scripts/fuzz_access.baseline.json] [--update-baseline]

Needs `javac` (JDK 21+) on PATH and builds grounds with cargo. Exit status 1
if any (seed, case) is a false positive the baseline does not list.
"""

import argparse
import json
import os
import random
import shutil
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from fuzz_overloads import REPO, grounds_check, javac_run  # noqa: E402

VALUES = {"int": "1", "long": "1L", "boolean": "true", "String": '"s"', "Object": "new Object()",
          "List<String>": "new ArrayList<String>()"}
TYPES = list(VALUES)
NAMES = ["m", "n", "f", "g"]
ACCESS = ["private", "", "protected", "public"]


class Member:
    def __init__(self, kind, name, ret, params=(), access="", static=False, abstract=False,
                 default=False, override=False):
        self.kind, self.name, self.ret, self.params = kind, name, ret, list(params)
        self.access, self.static, self.abstract, self.default, self.override = access, static, abstract, default, override

    def render(self, iface=False):
        mods = [x for x in [self.access, "default" if self.default else "", "abstract" if self.abstract and not iface else "",
                            "static" if self.static else ""] if x]
        head = ("@Override " if self.override else "") + " ".join(mods + [self.ret]) + " " + self.name
        if self.kind == "field":
            return f"{head} = {value_for(self.ret)};"
        ps = ", ".join(f"{t} p{k}" for k, t in enumerate(self.params))
        if self.abstract:
            return f"{head}({ps});"
        body = "" if self.ret == "void" else f" return {value_for(self.ret)}; "
        return f"{head}({ps}) {{{body}}}"


def rtype(rng):
    return rng.choice(TYPES + ["void", "void"])


def params(rng):
    return [rng.choice(TYPES) for _ in range(rng.choice([0, 0, 1, 1, 2]))]


def sig(m):
    return (m.name, tuple(m.params)) if m.kind == "method" else ("field", m.name)


def add(rng, members, m):
    """Mostly keep a class free of duplicate members (javac: already defined)."""
    if any(sig(x) == sig(m) for x in members) and rng.random() < 0.97:
        return
    members.append(m)


def make_world(rng):
    w = {"I": [], "A": [], "B": [], "G": None, "a_implements_i": rng.random() < 0.4}
    for _ in range(rng.randint(1, 3)):
        kind = rng.choice(["abstract", "abstract", "default", "static", "private", "const"])
        name = rng.choice(NAMES)
        if kind == "const":
            add(rng, w["I"], Member("field", name, rng.choice(TYPES)))
        else:
            add(rng, w["I"], Member("method", name, rtype(rng), params(rng), access="private" if kind == "private" else "",
                               static=kind == "static", abstract=kind == "abstract", default=kind == "default"))
    for _ in range(rng.randint(1, 4)):
        name = rng.choice(NAMES)
        if rng.random() < 0.35:
            add(rng, w["A"], Member("field", name, rng.choice(TYPES), access=rng.choice(ACCESS), static=rng.random() < 0.3))
        else:
            abstract = rng.random() < 0.3
            add(rng, w["A"], Member("method", name, rtype(rng), params(rng),
                               access=rng.choice(ACCESS[1:] if abstract else ACCESS),
                               static=not abstract and rng.random() < 0.25, abstract=abstract))
    # B: overrides of what it inherits (rarely with the wrong shape), always of
    # what is abstract unless a supertype implements it; then its own members
    inherited = [m for m in w["A"] + w["I"] if m.kind == "method" and not m.static and m.access != "private"]
    implemented = {sig(m) for m in w["A"] if m.kind == "method" and not m.abstract and not m.static and m.access == "public"}
    for m in inherited:
        must = m.abstract and not (m in w["I"] and sig(m) in implemented)
        if (must and rng.random() < 0.95) or rng.random() < 0.4:
            ret, ps = m.ret, list(m.params)
            r = rng.random()
            if r < 0.04:
                ret = rtype(rng)
            elif r < 0.08:
                ps = params(rng)
            access = "public" if m in w["I"] else rng.choice(ACCESS[ACCESS.index(m.access):])
            if rng.random() < 0.04:
                access = rng.choice(ACCESS)
            add(rng, w["B"], Member("method", m.name, ret, ps, access=access, override=rng.random() < 0.6))
    for _ in range(rng.randint(0, 2)):
        name = rng.choice(NAMES)
        if rng.random() < 0.4:
            add(rng, w["B"], Member("field", name, rng.choice(TYPES), access=rng.choice(ACCESS), static=rng.random() < 0.3))
        else:
            ps = params(rng)
            # a new overload, or a static that would hide an instance method (an error)
            clash = any(sig(x) == (name, tuple(ps)) for x in w["A"] + w["I"] if x.kind == "method")
            if clash and rng.random() < 0.9:
                continue
            add(rng, w["B"], Member("method", name, rtype(rng), ps, access=rng.choice(ACCESS),
                               static=rng.random() < 0.25, override=rng.random() < 0.05))
    if rng.random() < 0.15:
        w["B"].append(rng.choice([
            Member("method", "toString", "String", access="public", override=True),
            Member("method", "hashCode", "int", access="public", override=True),
            Member("method", "equals", "boolean", ["Object"], access="public", override=True),
            Member("method", "hashcode", "int", access="public", override=rng.random() < 0.3),
            Member("method", "equals", "boolean", ["B"], access="public", override=rng.random() < 0.3),
        ]))
    if rng.random() < 0.3:
        t = rng.choice(["String", "Integer", "List<String>"])
        w["G"] = t
        if rng.random() < 0.95:
            w["B"].append(Member("method", "take", "void", [t if rng.random() < 0.95 else "Object"], access="public",
                                 override=rng.random() < 0.7))
        if rng.random() < 0.95:
            w["B"].append(Member("method", "give", t if rng.random() < 0.9 else "Object", access="public",
                                 override=rng.random() < 0.7))
    return w


def value_for(t):
    return {"Integer": "1"}.get(t) or VALUES.get(t) or "null"


RECEIVERS = {
    "C": ["b", "a", "i", "new B()", "B", "A", "I", "((A) b)", "((I) b)"],
    "self": ["", "this", "super", "b", "B", "A", "I.super"],
    "lambda": ["", "this", "super"],
    "anon": ["", "this", "super", "B.this"],
    "inner": ["", "B.this", "B.super", "b"],
    "nested": ["", "b", "B", "new B()"],
    "local": ["", "B.this", "b"],
}


def plausible(recv, ctx, m, owner, w):
    """A cheap guess at whether `recv.m` compiles, to keep most cases valid."""
    if m.static:
        if owner == "I":
            return recv == "I"
        return recv in ({"A", "B"} if owner == "A" else {"B"}) or (recv == "" and ctx != "C")
    if recv in ("B", "A", "I"):
        return False
    if recv == "I.super":
        return owner == "I" and not w["a_implements_i"] and not m.abstract
    if recv in ("super", "B.super"):
        return owner == "A" and not m.abstract or (owner == "I" and w["a_implements_i"] and m.default)
    if recv in ("a", "((A) b)"):
        return owner == "A" or (owner == "I" and w["a_implements_i"])
    if recv in ("i", "((I) b)"):
        return owner == "I"
    if recv == "" and ctx in ("C", "nested"):
        return False
    return True


def make_site(rng, w, ctx, k, statics):
    owned = [(o, m) for o in ("I", "A", "B") for m in w[o] if m.name in NAMES]
    if ctx == "C" and statics and rng.random() < 0.5:
        # through the static import: a call or a field, never a method reference
        recv, name = "", rng.choice(statics)
        same = [m for _, m in owned if m.name == name]
        m = rng.choice(same) if same else None
    elif owned and rng.random() < 0.9:
        owner, m = rng.choice(owned)
        if m.access == "private" and rng.random() < 0.7:
            owner, m = rng.choice(owned)
        good = [r for r in RECEIVERS[ctx] if plausible(r, ctx, m, owner, w)]
        recv = rng.choice(good if good and rng.random() < 0.85 else RECEIVERS[ctx])
        name = m.name
    else:
        m, name, recv = None, rng.choice(NAMES + ["q"]), rng.choice(RECEIVERS[ctx])
    kind = m.kind if m is not None and rng.random() < 0.9 else rng.choice(["method", "field"])
    if kind == "field":
        target = rng.choice(["Object", "var", m.ret if m is not None and m.kind == "field" else rng.choice(TYPES)])
        qual = f"{recv}.{name}" if recv else name
        return f"{target} v{k} = {qual};"
    ps = m.params if m is not None and m.kind == "method" and rng.random() < 0.92 else params(rng)
    if rng.random() < 0.2 and not (ctx == "C" and recv == ""):
        # a method reference with a functional interface of the right shape, mostly
        if recv in ("", "I.super"):
            recv = "this" if recv == "" else recv
        boxed = {"int": "Integer", "long": "Long", "boolean": "Boolean"}
        bs = [boxed.get(t, t) for t in ps]
        unbound = recv == "B" and m is not None and not m.static
        args = (["B"] if unbound else []) + bs
        fi = {0: "Supplier<Object>" if m is not None and m.ret != "void" else "Runnable",
              1: f"Consumer<{args[0]}>" if len(args) == 1 else "",
              2: f"BiConsumer<{', '.join(args[:2])}>" if len(args) == 2 else ""}.get(len(args)) or "Runnable"
        if rng.random() < 0.15:
            fi = rng.choice(["Runnable", "Supplier<Object>", "Consumer<B>", "Function<B, Object>", "Consumer<String>"])
        return f"{fi} v{k} = {recv}::{name};"
    args = ", ".join(value_for(t) for t in ps)
    call = f"{recv}.{name}({args})" if recv else f"{name}({args})"
    ret = m.ret if m is not None and m.kind == "method" else "void"
    if ret == "void" or rng.random() < 0.5:
        return call + ";"
    target = rng.choice(["Object", "var", ret, ret] + ([rng.choice(TYPES)] if rng.random() < 0.2 else []))
    return f"{target} v{k} = {call};"


# members no source declares: record components, enum constants and methods,
# Object's methods on interfaces, arrays, anonymous classes through `var`;
# (valid, a near miss javac rejects)
IMPLICIT = [
    ("int v{k} = r.x();", "int v{k} = r.getX();"),
    ("String v{k} = r.y();", "String v{k} = r.y;"),
    ("boolean v{k} = r.equals(r);", "boolean v{k} = r.equals();"),
    ("int v{k} = r.hashCode() + R.of().x();", "int v{k} = R.x();"),
    ("String v{k} = r.toString();", "String v{k} = r.toStr();"),
    ("int v{k} = r.sum();", "int v{k} = r.sum(1);"),
    ("R v{k} = new R(1, \"s\");", "R v{k} = new R(1);"),
    ("E[] v{k} = E.values();", "E[] v{k} = e.values;"),
    ("E v{k} = E.valueOf(\"X\");", "E v{k} = E.valueOf(1);"),
    ("int v{k} = E.X.ordinal() + e.w();", "int v{k} = E.X.ordinal;"),
    ("String v{k} = e.name();", "String v{k} = e.name;"),
    ("int v{k} = e.compareTo(E.Y);", "int v{k} = e.compareTo(\"Y\");"),
    ("Class<?> v{k} = e.getDeclaringClass();", "Class<?> v{k} = E.getDeclaringClass();"),
    ("String v{k} = i.toString();", "String v{k} = I.toString();"),
    ("boolean v{k} = i.equals(i) && i.hashCode() > 0;", "boolean v{k} = i.equal(i);"),
    ("Class<?> v{k} = i.getClass();", "Class<?> v{k} = I.getClass();"),
    ("Object v{k} = j.toString() + j.hashCode();", "Object v{k} = j.clone();"),
    ("int v{k} = arr.length;", "int v{k} = arr.length();"),
    ("int[] v{k} = arr.clone();", "int v{k} = arr.size();"),
    ("String v{k} = strs[0].trim();", "String v{k} = strs.trim();"),
    ("var v{k} = new Object() {{ int z() {{ return 1; }} }}; int w{k} = v{k}.z();",
     "Object v{k} = new Object() {{ int z() {{ return 1; }} }}; int w{k} = v{k}.z();"),
    ("var v{k} = new Object() {{ String z = \"s\"; }}; String w{k} = v{k}.z;",
     "var v{k} = new Object() {{ String z = \"s\"; }}; String w{k} = v{k}.zz;"),
    ("int v{k} = new Object() {{ int z() {{ return 1; }} }}.z();", "int v{k} = new Object() {{ }}.z();"),
    ("Object v{k} = new B.Ne(); int w{k} = B.Ne.CNT + new B.Ne().cnt();", "int v{k} = B.CNT;"),
    ("Runnable v{k} = r::sum;", "Runnable v{k} = R::sum;"),
    ("Function<R, Integer> v{k} = R::x;", "Function<R, Integer> v{k} = R::z;"),
    ("Supplier<E[]> v{k} = E::values;", "Supplier<E> v{k} = E::values;"),
    ("Function<String, E> v{k} = E::valueOf;", "Function<Integer, E> v{k} = E::valueOf;"),
    ("IntFunction<int[]> v{k} = int[]::new;", "IntFunction<int[]> v{k} = int[]::make;"),
    ("Object v{k} = j.jm() + J.sm2();", "Object v{k} = j.sm2();"),
    ("Iterable<E> v{k} = List.of(E.values()); v{k}.forEach(x -> x.w());", "List.of(E.values()).forEach(x -> x.ww());"),
]


def make_case(rng):
    w = make_world(rng)
    statics = []
    imports = []
    if rng.random() < 0.3:
        # a static member, also inherited ones (B.x for A's static x), mostly
        cands = [(o, m.name) for o in ("A", "B", "I") for m in w[o] if m.static or (o == "I" and m.kind == "field")]
        cands += [("B", n) for o, n in cands if o == "A"]
        owner, name = rng.choice(cands) if cands and rng.random() < 0.85 else (rng.choice(["A", "B", "I"]), rng.choice(NAMES))
        imports.append((owner, name))
        statics.append(name)
    sites = []
    for k in range(rng.randint(1, 3)):
        if rng.random() < 0.25:
            good, near = rng.choice(IMPLICIT)
            sites.append(("C", (good if rng.random() < 0.8 else near).format(k=k)))
            continue
        ctx = rng.choice(["C", "C", "self", "lambda", "anon", "inner", "nested", "local"])
        sites.append((ctx, make_site(rng, w, ctx, k, statics)))
    return w, imports, sites


def render(case, pkg):
    w, imports, sites = case
    by = {}
    for ctx, s in sites:
        by.setdefault(ctx, []).append(s)
    L = [f"package {pkg};", "import java.util.*;", "import java.util.function.*;"]
    L += [f"import static {pkg}.{o}.{n};" for o, n in imports]
    L.append("interface I {")
    L += ["    " + m.render(iface=True) for m in w["I"]]
    L.append("}")
    if w["G"]:
        L.append("interface G<T> { void take(T t); T give(); }")
    L.append("abstract class A" + (" implements I" if w["a_implements_i"] else "") + " {")
    L += ["    " + m.render() for m in w["A"]]
    L.append("}")
    ifaces = ([] if w["a_implements_i"] else ["I"]) + ([f"G<{w['G']}>"] if w["G"] else [])
    L.append("class B extends A" + (" implements " + ", ".join(ifaces) if ifaces else "") + " {")
    L += ["    " + m.render() for m in w["B"]]
    L.append("    void self(B b) {")
    L += ["        " + s for s in by.get("self", [])]
    if by.get("lambda"):
        L.append("        Runnable zzL = () -> {")
        L += ["            " + s for s in by["lambda"]]
        L.append("        };")
    if by.get("anon"):
        L.append("        Object zzA = new B() {")
        L.append("            void zz() {")
        L += ["                " + s for s in by["anon"]]
        L.append("            }")
        L.append("        };")
    if by.get("local"):
        L.append("        class Zl {")
        L.append("            void zz() {")
        L += ["                " + s for s in by["local"]]
        L.append("            }")
        L.append("        }")
    L.append("    }")
    if by.get("inner"):
        L.append("    class In {")
        L.append("        void zz(B b) {")
        L += ["            " + s for s in by["inner"]]
        L.append("        }")
        L.append("    }")
    L.append("    static class Ne {")
    L.append("        static final int CNT = 1;")
    L.append("        int cnt() { return CNT; }")
    L.append("        void zz(B b) {")
    L += ["            " + s for s in by.get("nested", [])]
    L.append("        }")
    L.append("    }")
    L.append("}")
    L.append("interface J extends I { default String jm() { return \"j\"; } static int sm2() { return 2; } }")
    L.append("record R(int x, String y) {")
    L.append("    R { if (x < 0) throw new IllegalArgumentException(); }")
    L.append("    static R of() { return new R(1, \"s\"); }")
    L.append("    int sum() { return x + y.length(); }")
    L.append("}")
    L.append("enum E { X, Y; int w() { return ordinal(); } }")
    L.append("class C {")
    L.append("    void use(B b, A a, I i, J j, R r, E e, int[] arr, String[] strs) {")
    L += ["        " + s for s in by.get("C", [])]
    L.append("    }")
    L.append("}")
    return "\n".join(L) + "\n"


def javac_errors(paths, out, chunk=250):
    """{file name: [lines with an error]} (None for a file javac crashed on)."""
    from concurrent.futures import ThreadPoolExecutor
    chunks = [paths[i:i + chunk] for i in range(0, len(paths), chunk)]
    errs, crashed = {}, []
    with ThreadPoolExecutor(os.cpu_count() or 4) as ex:
        for c, (e, crash) in zip(chunks, ex.map(lambda ic: javac_run(ic[1], out, f"c{ic[0]}"), enumerate(chunks))):
            if crash:
                crashed += c
            else:
                errs.update(e)
        for p, (e, crash) in zip(crashed, ex.map(lambda p: javac_run([p], out, "s" + os.path.basename(p)[:-5]), crashed)):
            errs[os.path.basename(p)] = None if crash else e.get(os.path.basename(p), [])
    return {os.path.basename(p): (None if errs.get(os.path.basename(p), []) is None
                                  else sorted({line for line, _, _ in errs.get(os.path.basename(p), [])})) for p in paths}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--cases", type=int, default=2000)
    ap.add_argument("--out")
    ap.add_argument("--baseline", default=os.path.join(REPO, "scripts", "fuzz_access.baseline.json"))
    ap.add_argument("--update-baseline", action="store_true")
    a = ap.parse_args()
    out = os.path.abspath(a.out or os.path.join(REPO, "target", "fuzz-access", f"seed-{a.seed}"))
    shutil.rmtree(out, ignore_errors=True)
    cases_dir = os.path.join(out, "cases")
    os.makedirs(cases_dir)
    paths = []
    for i in range(1, a.cases + 1):
        name = f"a{a.seed}_{i:05d}"
        path = os.path.join(cases_dir, name + ".java")
        with open(path, "w") as fh:
            fh.write(render(make_case(random.Random(f"{a.seed}:{i}")), name))
        paths.append(path)
    jv = javac_errors(paths, out)
    findings = grounds_check(cases_dir)
    # suspect: in a file javac rejects, a finding on a line javac has no error
    # on; javac leaves out errors that follow from another one, so this is
    # only a lead, not a failure
    counts = {"accepted": 0, "rejected": 0, "javac-crash": 0, "false-error": 0, "caught": 0, "suspect": 0}
    suspects = []
    bad = []
    for path in paths:
        f = os.path.basename(path)
        lines, fs = jv[f], findings.get(f, [])
        if lines is None:
            counts["javac-crash"] += 1
        elif not lines:
            counts["accepted"] += 1
            if fs:
                counts["false-error"] += 1
                bad.append({"case": f[:-5], "findings": [f"{ln}: {r}: {m}" for ln, r, m in fs]})
        else:
            counts["rejected"] += 1
            if any(ln in lines for ln, _, _ in fs):
                counts["caught"] += 1
            off = [f"{ln}: {r}: {m}" for ln, r, m in fs if ln not in lines]
            if off:
                counts["suspect"] += 1
                suspects.append({"case": f[:-5], "findings": off, "javac_lines": lines})
    key = f"{a.seed}:{a.cases}"
    try:
        with open(a.baseline) as fh:
            base = json.load(fh)
    except (FileNotFoundError, json.JSONDecodeError):
        base = {}
    known = set(base.get(key, []))
    now = sorted(b["case"] for b in bad)
    new = [c for c in now if c not in known]
    fixed = sorted(c for c in known if c not in now)
    with open(os.path.join(out, "report.json"), "w") as fh:
        json.dump({"seed": a.seed, "counts": counts, "false_errors": bad, "suspects": suspects}, fh, indent=1)
    if a.update_baseline:
        base[key] = now
        with open(a.baseline, "w") as fh:
            json.dump(base, fh, indent=1, sort_keys=True)
            fh.write("\n")
    print(f"seed {a.seed}: " + ", ".join(f"{k} {v}" for k, v in counts.items()))
    for b in bad:
        if b["case"] in new:
            print(f"FALSE ERROR {b['case']} (javac accepts it): {'; '.join(b['findings'])}  ({cases_dir}/{b['case']}.java)")
    if fixed:
        print(f"no longer false errors (update the baseline): {', '.join(fixed)}")
    return 1 if new and not a.update_baseline else 0


if __name__ == "__main__":
    sys.exit(main())
