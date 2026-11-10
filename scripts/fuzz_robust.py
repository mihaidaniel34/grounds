#!/usr/bin/env python3
"""Robustness fuzzer: grounds must survive broken and hostile input.

grounds runs after every edit an agent makes, so it constantly sees files that
are half written, and projects with odd build files and jars. This makes small
projects in a temporary directory and runs the real binary on each:

  grounds check --root DIR --format json      (grounds.toml: scan_caches = false)

Three families of cases:

  syntax   a seed file (crates/grounds-rules/tests/fixtures/*.java and the other
           Java files under crates/*/tests) broken the way an editor leaves it:
           cut off at a random byte, a token deleted or duplicated, unbalanced
           braces/parentheses/angle brackets, an unterminated string, text
           block or comment, a method body replaced by garbage, two files
           spliced, invalid UTF-8, NUL bytes, a byte order mark, CRLF or lone
           CR line ends, an empty file, only comments, an enormous line or
           string literal.
  extreme  valid but extreme programs: nesting 500 deep (blocks, parentheses,
           ternaries, call chains, generic type arguments), thousands of
           fields, methods, overloads or switch cases, long chains of locals
           and of static final constants across classes (with cycles), cyclic
           inheritance, classes nested 100 deep, `$` and non-ASCII names,
           duplicate classes, a package that does not match its directory.
  project  broken project files: an empty or truncated jar, a zip that is not
           a jar, a jar with a corrupt class file (all listed in grounds.toml
           `[classpath] entries`), malformed grounds.toml, pom.xml,
           build.gradle, settings.gradle and lombok.config, a symlink loop in
           the source tree, a directory named like a Java file.

A failure is a panic (on stderr), a non-zero exit that is not a clean error
message (exit 1 with `Error: ...`, or 1/2 for findings), or running over the
time limit. A sample of cases also goes through the daemon: the case becomes a
git repository, gets an uncommitted edit, `grounds changed` runs twice (the
first starts the daemon, the second asks it) and `grounds daemon --stop` ends
it; there a daemon answering "internal error" (a caught panic) is a failure too.

Deterministic: a seed fixes the cases. Case i of seed s uses Random("s:i").

  python3 -I scripts/fuzz_robust.py --seed 1 --cases 3000 [--family syntax,extreme,project]
      [--timeout 20] [--daemon-every 25] [--keep DIR] [--jobs N] [--case N]

Builds grounds with cargo (or uses $GROUNDS_BIN). Exit status 1 if any case
failed. Failing cases are copied to --keep (default target/fuzz-robust/seed-S)
with the command to rerun them; `--case N` reruns only case N of the seed.
"""

import argparse
import glob
import io
import json
import os
import random
import re
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
import zipfile
from concurrent.futures import ThreadPoolExecutor

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FAMILIES = ["syntax", "extreme", "project"]
TOML = "[classpath]\nscan_caches = false\n"


# ---------------------------------------------------------------- seeds

def load_seeds():
    paths = sorted(glob.glob(os.path.join(REPO, "crates", "grounds-rules", "tests", "fixtures", "*.java")))
    paths += sorted(p for p in glob.glob(os.path.join(REPO, "crates", "*", "tests", "**", "*.java"), recursive=True)
                    if p not in set(paths))
    seeds = []
    for p in paths:
        with open(p, "rb") as fh:
            data = fh.read()
        if data.strip():
            seeds.append(data)
    if not seeds:
        sys.exit("no seed files found")
    return seeds


TOKEN_RE = re.compile(rb'"(?:\\.|[^"\\\n])*"|[A-Za-z_$][A-Za-z0-9_$]*|\d+[\w.]*|->|::|&&|\|\||[<>=!]=|\S')


def tokens(data):
    return [(m.start(), m.end()) for m in TOKEN_RE.finditer(data)]


def package_and_name(data):
    """(package or None, a type name declared in the file) of a seed."""
    m = re.search(rb"^\s*package\s+([\w.]+)\s*;", data, re.M)
    n = re.search(rb"\b(?:class|interface|enum|record)\s+([A-Za-z_$][\w$]*)", data)
    return (m.group(1).decode() if m else None), (n.group(1).decode() if n else "X")


# ---------------------------------------------------------------- syntax family

def m_truncate(rng, d, seeds):
    return d[:rng.randrange(len(d) + 1)]


def m_delete_token(rng, d, seeds):
    t = tokens(d)
    if not t:
        return d
    for _ in range(rng.randint(1, 3)):
        a, b = rng.choice(t)
        d = d[:a] + d[b:]
        t = tokens(d) or [(0, 0)]
    return d


def m_duplicate_token(rng, d, seeds):
    t = tokens(d)
    if not t:
        return d
    a, b = rng.choice(t)
    return d[:b] + b" " + d[a:b] + d[b:]


def m_unbalanced(rng, d, seeds):
    for _ in range(rng.randint(1, 4)):
        pos = rng.randrange(len(d) + 1)
        ch = rng.choice([b"{", b"}", b"(", b")", b"<", b">", b"[", b"]", b"<<<", b"}}}", b"(((", b">>>"])
        if rng.random() < 0.5:
            d = d[:pos] + ch + d[pos:]
        else:
            # remove one bracket of the file
            idx = [i for i, c in enumerate(d) if c in b"{}()<>[]"]
            if idx:
                i = rng.choice(idx)
                d = d[:i] + d[i + 1:]
    return d


def m_unterminated(rng, d, seeds):
    pos = rng.randrange(len(d) + 1)
    s = rng.choice([b'"', b'"abc', b"'", b"'\\", b'"""', b'"""\n  text', b"/*", b"/** doc", b'"\\u00', b"'\\u",
                    b'"\\', b"\\u000a", b"\\uuuu", b"@"])
    return d[:pos] + s + d[pos:]


def m_garbage_body(rng, d, seeds):
    bodies = [m.end() for m in re.finditer(rb"\)\s*(?:throws[\w\s,.]*)?\{", d)]
    if not bodies:
        return d
    start = rng.choice(bodies)
    depth, end = 1, start
    while end < len(d) and depth:
        depth += {ord("{"): 1, ord("}"): -1}.get(d[end], 0)
        end += 1
    n = rng.randint(1, 400)
    if rng.random() < 0.5:
        g = bytes(rng.randrange(256) for _ in range(n))
    else:
        pool = [b"if", b"(", b")", b"{", b"}", b"->", b"x", b".", b";", b"new", b"<", b">", b"?", b":", b"::", b"=", b"\"",
                b"switch", b"case", b"yield", b"var", b"@", b"[", b"]", b"0x", b"1e", b"'", b"/*", b"//", b"\n", b"class",
                b"return", b"this", b"super", b"...", b"instanceof", b"default", b"record", b"sealed", b"permits"]
        g = b" ".join(rng.choice(pool) for _ in range(n // 3 + 1))
    return d[:start] + g + d[max(start, end - 1):]


def m_splice(rng, d, seeds):
    o = rng.choice(seeds)
    return d[:rng.randrange(len(d) + 1)] + o[rng.randrange(len(o) + 1):]


def m_invalid_utf8(rng, d, seeds):
    for _ in range(rng.randint(1, 5)):
        pos = rng.randrange(len(d) + 1)
        b = rng.choice([b"\xff", b"\xc3", b"\xe2\x82", b"\xed\xa0\x80", b"\xf4\x90\x80\x80", b"\x80\x80",
                        bytes(rng.randrange(128, 256) for _ in range(rng.randint(1, 8)))])
        d = d[:pos] + b + d[pos:]
    return d


def m_nul(rng, d, seeds):
    for _ in range(rng.randint(1, 5)):
        pos = rng.randrange(len(d) + 1)
        d = d[:pos] + b"\x00" * rng.randint(1, 3) + d[pos:]
    return d


def m_bom(rng, d, seeds):
    return rng.choice([b"\xef\xbb\xbf", b"\xef\xbb\xbf\xef\xbb\xbf", b"\xfe\xff", b"\xff\xfe"]) + d


def m_line_ends(rng, d, seeds):
    r = rng.random()
    if r < 0.4:
        return d.replace(b"\n", b"\r\n")
    if r < 0.8:
        return d.replace(b"\n", b"\r")
    return bytes(c if c != 10 else rng.choice([10, 13, 13]) for c in d) + b"\r\n\r"


def m_empty(rng, d, seeds):
    return rng.choice([b"", b" ", b"\n", b"\t\n\r\n", b";", b"\xef\xbb\xbf"])


def m_only_comments(rng, d, seeds):
    parts = []
    for _ in range(rng.randint(1, 20)):
        parts.append(rng.choice([b"// a comment\n", b"/* block */\n", b"/** javadoc {@link Foo#bar(int)} */\n",
                                 b"/* /* not nested */\n", b"//\n", b"/**/", b"// \\u000a not a newline\n"]))
    return b"".join(parts)


def m_enormous_line(rng, d, seeds):
    n = rng.choice([100_000, 1_000_000, 3_000_000])
    filler = rng.choice([b" ", b"a", b";", b"/**/", b"x."])
    line = filler * (n // len(filler))
    pos = rng.randrange(len(d) + 1)
    return d[:pos] + line + d[pos:]


def m_enormous_string(rng, d, seeds):
    n = rng.choice([100_000, 1_000_000, 3_000_000])
    body = rng.choice([b"a", b"\\n", b"\\u0041", b"\\\""]) * (n // 2)
    lit = (b'"""\n' + body + b'"""') if rng.random() < 0.3 else (b'"' + body + b'"')
    return d.replace(b"{", b"{ String zz_big = " + lit + b";", 1) if b"class" in d else lit


MUTATORS = [m_truncate, m_delete_token, m_duplicate_token, m_unbalanced, m_unterminated, m_garbage_body, m_splice,
            m_invalid_utf8, m_nul, m_bom, m_line_ends, m_empty, m_only_comments, m_enormous_line, m_enormous_string]
# the enormous ones are slow to write and parse: keep them rarer
WEIGHTS = [6, 5, 4, 5, 5, 5, 4, 3, 3, 2, 3, 1, 1, 1, 1]


def case_syntax(rng, seeds):
    files = {}
    names = []
    for k in range(rng.choice([1, 1, 1, 2, 3])):
        d = rng.choice(seeds)
        muts = rng.choices(MUTATORS, WEIGHTS, k=rng.choice([1, 1, 1, 2, 3]))
        for m in muts:
            d = m(rng, d, seeds)
        names.append("+".join(m.__name__[2:] for m in muts))
        pkg, cls = package_and_name(rng.choice(seeds) if not d else d)
        # a mutation can make the "package" a million segments long
        cls = cls if len(cls) <= 60 else "X"
        sub = pkg.replace(".", "/") if pkg and len(pkg) <= 120 and rng.random() < 0.7 else "p"
        files[f"src/main/java/{sub}/{cls}{'' if k == 0 else k}.java"] = d
    return " ".join(names), {"files": files}


# ---------------------------------------------------------------- extreme family

def nest(n, open_, close_, inner):
    return open_ * n + inner + close_ * n


def x_blocks(rng):
    n = rng.choice([200, 500, 500])
    return f"deep blocks {n}", f"class C {{ void m() {{ {nest(n, '{ ', '} ', 'int x = 1;')} }} }}"


def x_parens(rng):
    n = rng.choice([200, 500, 500])
    return f"deep parentheses {n}", f"class C {{ int m(int a) {{ return {nest(n, '(', ')', 'a + 1')}; }} }}"


def x_ternary(rng):
    n = rng.choice([200, 500, 500])
    if rng.random() < 0.5:
        e = "0"
        for i in range(n):
            e = f"a > {i} ? {i} : {e}" if rng.random() < 0.5 else f"(a > {i} ? {e} : {i})"
    else:
        e = " ".join(f"a > {i} ? {i} :" for i in range(n)) + " -1"
    return f"deep ternary {n}", f"class C {{ int m(int a) {{ return {e}; }} }}"


def x_call_chain(rng):
    n = rng.choice([200, 500, 500])
    kind = rng.choice(["builder", "nested-args", "stream", "string"])
    if kind == "builder":
        e = "new StringBuilder()" + ".append(1)" * n + ".toString()"
    elif kind == "nested-args":
        e = nest(n, "f(", ")", "1")
    elif kind == "stream":
        e = "java.util.stream.Stream.of(1)" + ".map(x -> x + 1)" * n + ".count()"
    else:
        e = '"a"' + ".trim()" * n
    body = f"Object m() {{ return {e}; }} static int f(int x) {{ return x; }}"
    if rng.random() < 0.3:
        body += f" void h() {{ assertThat(1, {nest(min(n, 300), 'is(', ')', '1')}); }}" \
                " static <T> T is(T t) { return t; } static void assertThat(Object a, Object b) {}"
    return f"call chain {kind} {n}", f"class C {{ {body} }}"


def x_generics(rng):
    n = rng.choice([200, 500, 500])
    t = nest(n, "java.util.List<", ">", "String")
    if rng.random() < 0.5:
        return f"deep generic arguments {n}", f"class C {{ {t} f; {t} m({t} a) {{ return a; }} }}"
    return f"deep generic arguments in new {n}", f"class C {{ Object f = new java.util.ArrayList<{t}>(); }}"


def x_lambda(rng):
    n = rng.choice([200, 500])
    e = nest(n, "() -> ", "", "1")
    return f"deep lambdas {n}", f"class C {{ Object m() {{ java.util.function.Supplier<Object> s = {e}; return s; }} }}"


def x_binary(rng):
    n = rng.choice([2000, 10000, 50000])
    op = rng.choice(["+", "&&", "+ \"s\" +", "*", "|"])
    t = {"&&": "boolean", "+ \"s\" +": "String"}.get(op, "int")
    v = {"&&": "a"}.get(op, "1")
    return f"long binary chain {op} {n}", f"class C {{ {t} m(boolean a) {{ return {(' ' + op + ' ').join([v] * n)}; }} }}"


def x_if_else(rng):
    n = rng.choice([500, 2000])
    s = "if (a == 0) return 0;" + "".join(f" else if (a == {i}) return {i};" for i in range(1, n))
    return f"else-if chain {n}", f"class C {{ int m(int a) {{ {s} return -1; }} }}"


def x_members(rng):
    n = rng.choice([1000, 5000])
    kind = rng.choice(["fields", "methods", "overloads", "constants"])
    if kind == "fields":
        body = "\n".join(f"int f{i} = {i};" for i in range(n))
    elif kind == "methods":
        body = "\n".join(f"int m{i}() {{ return m{(i + 1) % n}(); }}" for i in range(n))
    elif kind == "overloads":
        n = min(n, 2000)
        types = ["int", "long", "String", "Object", "Integer", "double", "char", "byte"]
        sigs = set()
        lines = []
        r = random.Random(n)
        while len(lines) < n:
            ps = tuple(r.choice(types) for _ in range(r.randint(0, 4)))
            if ps in sigs:
                continue
            sigs.add(ps)
            lines.append(f"void m({', '.join(f'{t} p{k}' for k, t in enumerate(ps))}) {{}}")
        lines.append("void use() { m(); m(1); m(1, 2); m(\"a\", null); m(null, null, null); m('c', 1L, 2.0, (byte) 1); }")
        body = "\n".join(lines)
    else:
        body = "\n".join(f"static final int K{i} = K{i - 1} + 1;" if i else "static final int K0 = 1;" for i in range(n))
    return f"{n} {kind}", f"class C {{\n{body}\n}}"


def x_switch(rng):
    n = rng.choice([1000, 3000])
    if rng.random() < 0.5:
        cases = "\n".join(f"case {i}: return {i};" for i in range(n))
        return f"switch {n} cases", f"class C {{ int m(int a) {{ switch (a) {{\n{cases}\ndefault: return -1; }} }} }}"
    cases = "\n".join(f"case \"s{i}\" -> {i};" for i in range(n))
    return f"switch expression {n} cases", f"class C {{ int m(String a) {{ return switch (a) {{\n{cases}\ndefault -> -1; }}; }} }}"


def x_local_chain(rng):
    n = rng.choice([500, 2000, 5000])
    lines = ["int v0 = 1;"] + [f"int v{i} = v{i - 1} {rng.choice(['+', '*', '-', '<<', '|'])} v{i - 1};" for i in range(1, n)]
    if rng.random() < 0.5:
        lines = ["final " + x for x in lines]
    ret = f"return v{n - 1};"
    if rng.random() < 0.3:
        # reassignments a = b; b = c; ... and back
        lines = [f"int v{i} = {i};" for i in range(n)] + [f"v{i} = v{i + 1};" for i in range(n - 1)] + [f"v{n - 1} = v0;"]
    return f"local chain {n}", "class C { int m() {\n" + "\n".join(lines) + f"\n{ret} }} }}"


def x_const_chain(rng):
    """Static final constants defined by another class's, across many classes, with cycles."""
    n = rng.choice([50, 300, 1000])
    cyc = rng.random() < 0.5
    files = {}
    op = rng.choice(["+", "*", "+ \"x\" +"])
    for i in range(n):
        prev = f"K{i - 1}.V" if i else (f"K{n - 1}.V" if cyc else "1")
        twice = f"{prev} {op} {prev}" if rng.random() < 0.7 else prev
        if op == "+ \"x\" +":
            decl = f"static final String V = {twice};"
        else:
            decl = f"static final int V = {twice};"
        files[f"src/main/java/p/K{i}.java"] = f"package p;\npublic class K{i} {{ {decl} }}\n"
    files["src/main/java/p/Use.java"] = (f"package p;\nclass Use {{ Object m(int a) {{ if (K{n - 1}.V == 3) return null; "
                                         f"switch (a) {{ case K0.V: return 1; }} return K{n - 1}.V; }} }}\n")
    return f"constant chain {n} {'cyclic' if cyc else 'acyclic'} {op}", files


def x_const_cycle_two(rng):
    a = rng.choice(["B.Y + 1", "B.Y", "B.Y * 2 + A.X"])
    return "constant cycle between two classes", {
        "src/main/java/p/A.java": f"package p;\nclass A {{ static final int X = {a}; int m() {{ return X > 0 ? 1 : 0; }} }}\n",
        "src/main/java/p/B.java": "package p;\nclass B { static final int Y = A.X + 1; boolean n() { return Y == A.X; } }\n",
    }


def x_inheritance_cycle(rng):
    kind = rng.choice(["two", "self", "iface", "long", "generic", "nested", "mixed"])
    if kind == "two":
        s = "class A extends B { void m() { super.m(); n(); } } class B extends A { void n() { m(); } }"
    elif kind == "self":
        s = "class A extends A { void m() { super.toString(); this.m(); } @Override public String toString() { return null; } }"
    elif kind == "iface":
        s = "interface I extends J { void a(); } interface J extends K { void b(); } interface K extends I { } " \
            "class C implements I { public void a() {} } class D extends C implements K { void x() { a(); b(); } }"
    elif kind == "long":
        n = 300
        s = " ".join(f"class A{i} extends A{(i + 1) % n} {{ void m{i}() {{ m{(i + 7) % n}(); }} }}" for i in range(n))
    elif kind == "generic":
        s = "class A<T extends B<T>> extends B<A<T>> {} class B<U extends A<U>> extends A<B<U>> { void m(U u) { u.toString(); } }"
    elif kind == "nested":
        s = "class O extends O.I { static class I extends O { } void m() { new I().m(); } }"
    else:
        s = "interface I extends I { default void m() { m(); } } class A implements I, I { } enum E implements E {}" \
            " record R(R r) implements R {} class S extends Object, Object {}"
    return f"inheritance cycle {kind}", s


def x_nested_classes(rng):
    n = rng.choice([50, 100, 100])
    kind = rng.choice(["static", "inner", "anon", "local"])
    if kind in ("static", "inner"):
        mod = "static " if kind == "static" else ""
        inner = "int f; void m() { f++; }"
        s = "".join(f"{mod if i else ''}class N{i} {{ " for i in range(n)) + inner + " }" * n
    elif kind == "anon":
        s = "class C { Object m() { return " + nest(n, "new Object() { Object x() { return ", "; } }", "null") + "; } }"
    else:
        s = "class C { void m() { " + "".join(f"class L{i} {{ void m() {{ " for i in range(n)) + "int x = 1;" \
            + " } }" * n + " } }"
    return f"{n} nested classes {kind}", s


def x_identifiers(rng):
    names = ["$", "_$", "$$", "a$b", "Ünïcödé", "名前", "π", "café", "Δx", "ℌ", "\U0001d49c", "x\u200dy", "ſ", "ǅ",
             "a\u0301", "$1", "Foo$Bar", "Outer$Inner"]
    picks = rng.sample(names, k=6)
    cls = rng.choice(["C$1", "Ñ", "Foo$Bar", "名前", "$"])
    body = "\n".join(f"int {n} = {i}; int {n}m() {{ return {n}; }}" for i, n in enumerate(picks))
    use = " + ".join(f"{n}m()" for n in picks)
    return f"identifiers {' '.join(picks)}", {
        f"src/main/java/p/{cls}.java": f"package p;\npublic class {cls} {{\n{body}\nint all() {{ return {use}; }}\n"
                                      f"class In$ner {{ }} static class 内 {{ }} }}\n"}


def x_duplicates(rng):
    kind = rng.choice(["same-file", "two-files", "members", "package-mismatch", "package-and-class"])
    if kind == "same-file":
        return "duplicate classes in one file", "class A { int x; } class A { String x; void m() { x.length(); } }"
    if kind == "two-files":
        return "duplicate classes across files", {
            "src/main/java/p/A.java": "package p;\npublic class A { public int m() { return 1; } }\n",
            "src/main/java/p/B.java": "package p;\nclass A { String m() { return null; } } class B { int n() { return new A().m(); } }\n",
            "src/test/java/p/A.java": "package p;\nclass A extends A { }\n"}
    if kind == "members":
        return "duplicate members", "class A { int x; String x; void m() {} int m() { return 0; } A() {} A() {} " \
                                    "void u() { int y = x; m(); new A(); } }"
    if kind == "package-mismatch":
        return "package does not match directory", {
            "src/main/java/p/A.java": "package q.r;\npublic class A { public static int K = 1; }\n",
            "src/main/java/q/r/B.java": "package q.r;\nclass B { int m() { return A.K; } }\n",
            "src/main/java/z/C.java": "package p;\nimport q.r.A;\nclass C { int m() { return A.K + new B().m(); } }\n"}
    return "package named like a class", {
        "src/main/java/p/A.java": "package p;\npublic class A { public static class B { } }\n",
        "src/main/java/p/A/B.java": "package p.A;\npublic class B { }\n",
        "src/main/java/q/U.java": "package q;\nimport p.A.B;\nclass U { B b; }\n"}


def x_wide_file(rng):
    """Many small classes in one file, all referencing each other."""
    n = rng.choice([1000, 3000])
    s = "\n".join(f"class T{i} extends T{i + 1} {{ T{(i * 7) % n} f; }}" for i in range(n - 1)) + f"\nclass T{n - 1} {{}}"
    return f"{n} classes in one file", s


EXTREME = [x_blocks, x_parens, x_ternary, x_call_chain, x_generics, x_lambda, x_binary, x_if_else, x_members,
           x_switch, x_local_chain, x_const_chain, x_const_cycle_two, x_inheritance_cycle, x_nested_classes,
           x_identifiers, x_duplicates, x_wide_file]


def case_extreme(rng, seeds):
    g = rng.choice(EXTREME)
    name, src = g(rng)
    if isinstance(src, str):
        src = {"src/main/java/p/C.java": "package p;\n" + src + "\n"}
    return f"{g.__name__[2:]}: {name}", {"files": {k: v.encode("utf-8") for k, v in src.items()}}


# ---------------------------------------------------------------- project family

def class_file(name, sup="java/lang/Object", methods=True):
    """A minimal valid class file (Java 8) with an empty constructor-less body."""
    cp = []

    def utf8(s):
        b = s.encode()
        cp.append(b"\x01" + struct.pack(">H", len(b)) + b)
        return len(cp)

    def cls(s):
        i = utf8(s)
        cp.append(b"\x07" + struct.pack(">H", i))
        return len(cp)

    this = cls(name)
    supr = cls(sup)
    mname, mdesc = utf8("m"), utf8("(Ljava/lang/String;)I")
    fname, fdesc = utf8("f"), utf8("Ljava/util/List;")
    out = b"\xca\xfe\xba\xbe" + struct.pack(">HHH", 0, 52, len(cp) + 1) + b"".join(cp)
    out += struct.pack(">HHH", 0x0021, this, supr) + struct.pack(">H", 0)
    out += struct.pack(">H", 1) + struct.pack(">HHHH", 0x0001, fname, fdesc, 0)
    out += struct.pack(">H", 1 if methods else 0)
    if methods:
        out += struct.pack(">HHHH", 0x0401, mname, mdesc, 0)
    out += struct.pack(">H", 0)
    return out


def corrupt_class(rng, data):
    r = rng.random()
    if r < 0.25:
        return data[:rng.randrange(len(data))]
    if r < 0.45:
        # constant pool count far beyond the data
        return data[:8] + struct.pack(">H", rng.choice([0, 1, 0xffff, 40000])) + data[10:]
    if r < 0.6:
        return b"\xca\xfe\xba\xbe" + bytes(rng.randrange(256) for _ in range(rng.randint(0, 300)))
    if r < 0.75:
        # utf8 entry claiming a huge length, or a bad tag
        i = 10
        return data[:i] + rng.choice([b"\x01\xff\xff", b"\x63", b"\x07\xff\xff", b"\x01\x00\x05\xed\xa0\x80\xc0\x80"]) + data[i + 3:]
    b = bytearray(data)
    for _ in range(rng.randint(1, 10)):
        b[rng.randrange(len(b))] = rng.randrange(256)
    return bytes(b)


def zip_bytes(entries):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        for n, d in entries:
            # a ZipInfo does not take the archive's compression: deflate explicitly
            zi = zipfile.ZipInfo(n, date_time=(2020, 1, 1, 0, 0, 0))
            zi.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(zi, d)
    return buf.getvalue()


USE_LIB = ("package p;\nimport lib.Lib;\nimport lib.*;\npublic class App extends Lib {\n"
           "    int m() { return new Lib().m(\"x\") + f.size(); }\n    @Override public int m(String s) { return 1; }\n}\n")


def p_jar(rng):
    kind = rng.choice(["empty", "truncated", "not-a-jar", "corrupt-class", "garbage", "dir-as-jar", "missing",
                       "zip-bomb-ish", "odd-names"])
    good = class_file("lib/Lib")
    files = {}
    if kind == "empty":
        jar = b""
    elif kind == "truncated":
        full = zip_bytes([("META-INF/MANIFEST.MF", b"Manifest-Version: 1.0\n"), ("lib/Lib.class", good)])
        jar = full[:rng.randrange(1, len(full))]
    elif kind == "not-a-jar":
        jar = zip_bytes([("readme.txt", b"hello"), ("lib/Lib.class/", b""), ("x/y.java", b"class y {}")])
    elif kind == "corrupt-class":
        jar = zip_bytes([("lib/Lib.class", corrupt_class(rng, good)), ("lib/Ok.class", class_file("lib/Ok", "lib/Lib")),
                         ("lib/Wrong.class", class_file("other/Name"))])
    elif kind == "garbage":
        jar = bytes(rng.randrange(256) for _ in range(rng.randint(1, 4000)))
        if rng.random() < 0.5:
            jar = b"PK\x03\x04" + jar
    elif kind == "zip-bomb-ish":
        jar = zip_bytes([("lib/Lib.class", good)] + [(f"lib/C{i}.class", b"\x00" * 100_000) for i in range(30)])
    elif kind == "odd-names":
        jar = zip_bytes([("../../evil.class", good), ("/abs/Lib.class", good), ("lib//Lib.class", good),
                         ("lib/Lib.class", good), ("lib/\xe9t\xe9.class", good), ("module-info.class", corrupt_class(rng, good))])
    else:
        jar = None
    if kind == "dir-as-jar":
        files["lib/x.jar/lib/Lib.class"] = corrupt_class(rng, good)
    elif jar is not None:
        files["lib/x.jar"] = jar
    entries = ['"lib/x.jar"'] + (['"lib/x.jar"', '"lib"', '"/nonexistent/y.jar"', '"."'] if rng.random() < 0.3 else [])
    files["grounds.toml"] = (TOML + f"entries = [{', '.join(entries)}]\n").encode()
    if rng.random() < 0.3:
        files["classpath.txt"] = b"lib/x.jar\n\n:::\n/nonexistent\nlib/x.jar:lib/x.jar\n\x00\n"
        files["grounds.toml"] += b'files = ["classpath.txt"]\n'
    files["src/main/java/p/App.java"] = USE_LIB.encode()
    return f"jar {kind}", files


def truncate_or_garble(rng, text):
    r = rng.random()
    if r < 0.4:
        return text[:rng.randrange(len(text) + 1)]
    if r < 0.6:
        return bytes(rng.randrange(256) for _ in range(rng.randint(0, 500)))
    if r < 0.8:
        b = bytearray(text)
        for _ in range(rng.randint(1, 6)):
            b.insert(rng.randrange(len(b) + 1), rng.choice(b"{}<>()[]\"'=\n\x00\\"))
        return bytes(b)
    return text


TOMLS = [
    b'[classpath]\nscan_caches = "yes"\n',
    b"[classpath]\nscan_caches = false\nentries = 3\n",
    b"java_release = -1\n[classpath]\nscan_caches = false\n",
    b"java_release = 99999999999\n[classpath]\nscan_caches = false\n",
    b"java_release = 4\n[classpath]\nscan_caches = false\n",
    b"java_release = 0\n[classpath]\nscan_caches = false\n",
    b'java_home = "/nonexistent"\n[classpath]\nscan_caches = false\n',
    b'java_home = ""\n[classpath]\nscan_caches = false\n',
    b'exclude = ["[", "**/*", "\\u0000"]\n[classpath]\nscan_caches = false\n',
    b'exclude = ["**"]\n[classpath]\nscan_caches = false\n',
    b'[rules]\nenable = ["nonexistent", ""]\n[classpath]\nscan_caches = false\n',
    b'[rules]\ndisable = ["all", "bug", "*"]\n[classpath]\nscan_caches = false\n',
    b'[rules.severity]\nnull-dereference = "fatal"\n"" = "error"\n[classpath]\nscan_caches = false\n',
    b'[rules.options.cognitive-complexity]\nmax = -5\n[rules.options.long-method]\nmax = "x"\n'
    b'[rules.options.too-many-parameters]\nmax = 1e400\n[classpath]\nscan_caches = false\n',
    b'[rules.options]\nmagic-number = [1, 2]\nline-length = {max = 0}\n[classpath]\nscan_caches = false\n',
    b'[report]\nmax_findings = 0\nmin_severity = "none"\n[classpath]\nscan_caches = false\n',
    b'[report]\nmax_findings = -1\n[classpath]\nscan_caches = false\n',
    b'[classpath]\nscan_caches = false\nfiles = ["/dev/zero"]\n',
    b'[classpath]\nscan_caches = false\nfiles = ["grounds.toml", "src", "nonexistent"]\n',
    b'[classpath]\nscan_caches = false\nentries = ["/", "..", "src/main/java", ""]\n',
    b'test_source_sets = ["main", "", "../.."]\n[classpath]\nscan_caches = false\n',
    b"[classpath]\nscan_caches = false\n[classpath]\nscan_caches = true\n",
    b"[[classpath]]\nscan_caches = false\n",
    b"\xef\xbb\xbf[classpath]\nscan_caches = false\n",
    b"[classpath\nscan_caches = false\n",
    b"[classpath]\nscan_caches = false\nunknown_key = 1\n[unknown]\nx = 1\n",
]


POMS = [
    b"",
    b"<project>",
    b"<project><modules><module>sub</module><module>.</module><module>..</module><module>/</module><module>sub/../sub</module></modules></project>",
    b"<project><properties><maven.compiler.release>abc</maven.compiler.release><maven.compiler.source>1.x</maven.compiler.source>"
    b"<java.version>${java.version}</java.version><maven.compiler.target>${a}</maven.compiler.target><a>${java.version}</a></properties></project>",
    b"<project><properties><maven.compiler.release>99999999999999999999</maven.compiler.release></properties></project>",
    b"<project><properties><maven.compiler.release>-3</maven.compiler.release></properties></project>",
    b'<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ENTITY lol2 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">'
    b'<!ENTITY lol3 "&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;&lol2;">]><project><name>&lol3;</name></project>',
    b"<project>" + b"<a>" * 5000 + b"</a>" * 5000 + b"</project>",
    b"<project><build><plugins><plugin><artifactId>maven-compiler-plugin</artifactId><configuration><release>${r}</release>"
    b"<source></source><compilerArgs><arg>--release</arg><arg></arg></compilerArgs></configuration></plugin></plugins></build></project>",
    b"<project><parent><relativePath>../../../../../../</relativePath></parent><dependencies><dependency><groupId/>"
    b"</dependency></dependencies></project>",
    b"<project><!-- unterminated comment",
    b"<project><![CDATA[ unterminated",
    b"<project xmlns='\x00'>\xff\xfe</project>",
]


GRADLES = [
    b"",
    b"plugins { id 'java'",
    b"plugins { id 'java' }\njava { toolchain { languageVersion = JavaLanguageVersion.of(abc) } }\n",
    b"plugins { id 'java' }\njava { toolchain { languageVersion = JavaLanguageVersion.of(999999999999999999999) } }\n",
    b"sourceCompatibility = '1.'\ntargetCompatibility = JavaVersion.VERSION_\n",
    b"sourceCompatibility = 0\noptions.release = -1\noptions.release.set(\n",
    b'tasks.withType(JavaCompile) { options.compilerArgs += ["--release", ] }\n',
    b"sourceSets { main { java { srcDirs = ['src', '..', '/', 'src/main/java/p/C.java'] } } }\n",
    b"/* unterminated\nplugins { id 'java' }\n",
    b"plugins { id 'java' }\n" + b"{" * 10000,
    b'def s = """\nunterminated\n',
    b"\xff\xfe\x00p\x00l\x00u\x00g",
]


SETTINGS = [
    b"",
    b"include ':a', ':b', ':a:b:c', '::', ':', '', ':..', ':../..', ':/'\n",
    b"include 'sub'\nproject(':sub').projectDir = file('..')\n",
    b"include 'sub'\nproject(':sub').projectDir = new File(rootDir, 'src')\n",
    b"includeBuild '.'\nincludeBuild '..'\ninclude(\n",
    b"rootProject.name = \nincludeFlat 'x'\n",
    b"include(" + b"'a'," * 5000 + b")\n",
    b"include '\x00'\n",
]


LOMBOKS = [
    b"",
    b"lombok.accessors.fluent = \nlombok.accessors.prefix += \nlombok.accessors.chain = maybe\n",
    b"config.stopBubbling = true\nlombok.copyableAnnotations += \nlombok.copyableAnnotations += ...\n",
    b"lombok.accessors.prefix += m_\nlombok.accessors.prefix -= m_\nlombok.accessors.prefix += \xff\xfe\n",
    b"import ../lombok.config\nimport lombok.config\nimport /dev/zero\n",
    b"lombok.anyConstructor.addConstructorProperties\n=\n==\n+=\n-=\n",
    b"clear lombok.accessors.prefix\nlombok.accessors.prefix += " + b"a" * 100_000 + b"\n",
    b"lombok.getter.noIsPrefix = true = false\nlombok.fieldDefaults.defaultPrivate = TRUE\n",
]


LOMBOK_SRC = ("package p;\nimport lombok.*;\n@Data @Builder @AllArgsConstructor @RequiredArgsConstructor\n"
              "public class L { private final String m_name; private int count; private boolean active;\n"
              "  static void use(L l) { l.getName(); l.name(); l.getM_name(); l.isActive(); L.builder().count(1).build(); } }\n")

APP = ("package p;\nimport java.util.*;\npublic class App {\n    List<String> xs = new ArrayList<>();\n"
       "    String m(String s) { if (s == null) return s.trim(); return s; }\n}\n")


def p_build_files(rng):
    which = rng.choice(["grounds.toml", "pom.xml", "build.gradle", "settings.gradle", "lombok.config", "mixed"])
    files = {"src/main/java/p/App.java": APP.encode(), "src/main/java/p/L.java": LOMBOK_SRC.encode()}
    choices = {"grounds.toml": TOMLS, "pom.xml": POMS, "build.gradle": GRADLES, "settings.gradle": SETTINGS,
               "lombok.config": LOMBOKS}
    picked = list(choices) if which == "mixed" else [which]
    for w in picked:
        content = rng.choice(choices[w])
        if rng.random() < 0.3:
            content = truncate_or_garble(rng, content)
        if w == "grounds.toml":
            files["grounds.toml"] = content
        elif w == "lombok.config":
            files[rng.choice(["lombok.config", "src/main/java/lombok.config", "src/main/java/p/lombok.config"])] = content
        else:
            files[w] = content
            if rng.random() < 0.5:
                files["sub/" + w] = rng.choice(choices[w])
                files["sub/src/main/java/s/S.java"] = b"package s;\npublic class S { p.App a; }\n"
            if w == "settings.gradle" and rng.random() < 0.5:
                files["build.gradle.kts"] = b"plugins { java }\njava { toolchain { languageVersion.set(JavaLanguageVersion.of(\"x\")) } }\n"
    files.setdefault("grounds.toml", TOML.encode())
    return f"build file {which}", files


def p_fs(rng):
    kind = rng.choice(["symlink-loop", "symlink-parent", "symlink-pair", "dir-named-java", "dangling-symlink",
                       "unreadable", "fifo", "deep-dirs", "java-symlink-to-dir", "same-file-twice"])
    files = {"grounds.toml": TOML.encode(), "src/main/java/p/App.java": APP.encode()}
    links, dirs, special = {}, [], []
    if kind == "symlink-loop":
        links["src/main/java/p/loop"] = "."
    elif kind == "symlink-parent":
        links["src/main/java/p/up"] = "../../../.."
    elif kind == "symlink-pair":
        links["src/main/java/a"] = "b"
        links["src/main/java/b"] = "a"
    elif kind == "dir-named-java":
        dirs.append("src/main/java/p/Foo.java")
        dirs.append("src/main/java/p/Bar.java/Baz.java")
        files["src/main/java/p/Bar.java/Baz.java/Q.java"] = b"package p;\nclass Q {}\n"
    elif kind == "dangling-symlink":
        links["src/main/java/p/Gone.java"] = "/nonexistent/Gone.java"
        links["build.gradle"] = "/nonexistent/build.gradle"
    elif kind == "unreadable":
        files["src/main/java/p/Secret.java"] = b"package p;\nclass Secret {}\n"
        special.append(("chmod000", "src/main/java/p/Secret.java"))
        files["src/main/java/q/Hidden.java"] = b"package q;\nclass Hidden {}\n"
        special.append(("chmod000", "src/main/java/q"))
    elif kind == "fifo":
        special.append(("fifo", "src/main/java/p/Pipe.java"))
        special.append(("fifo", "lombok.config"))
    elif kind == "deep-dirs":
        d = "src/main/java/" + "/".join(f"d{i}" for i in range(100))
        files[d + "/Deep.java"] = ("package " + ".".join(f"d{i}" for i in range(100)) + ";\npublic class Deep {}\n").encode()
    elif kind == "java-symlink-to-dir":
        links["src/main/java/p/Dir.java"] = "."
    else:
        links["src/main/java/p/Copy.java"] = "App.java"
        links["src/main/java/q"] = "p"
    return f"filesystem {kind}", files, links, dirs, special


def case_project(rng, seeds):
    r = rng.random()
    if r < 0.35:
        name, files = p_jar(rng)
        return name, {"files": files}
    if r < 0.75:
        name, files = p_build_files(rng)
        return name, {"files": files}
    name, files, links, dirs, special = p_fs(rng)
    return name, {"files": files, "links": links, "dirs": dirs, "special": special}


# ---------------------------------------------------------------- running

def make_case(seed, i, fam_names, seeds):
    rng = random.Random(f"{seed}:{i}")
    fam = rng.choice(fam_names)
    name, spec = {"syntax": case_syntax, "extreme": case_extreme, "project": case_project}[fam](rng, seeds)
    daemon = rng.random()
    return fam, name, spec, daemon


def materialize(spec, d):
    files = dict(spec["files"])
    files.setdefault("grounds.toml", TOML.encode())
    for rel in spec.get("dirs", []):
        os.makedirs(os.path.join(d, rel), exist_ok=True)
    for rel, data in files.items():
        p = os.path.join(d, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        if os.path.isdir(p):
            continue
        with open(p, "wb") as fh:
            fh.write(data)
    for rel, target in spec.get("links", {}).items():
        p = os.path.join(d, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        os.symlink(target, p)
    for op, rel in spec.get("special", []):
        p = os.path.join(d, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        if op == "fifo":
            os.mkfifo(p)
        elif op == "chmod000":
            os.chmod(p, 0)


def unlock(d):
    """Undo chmod 000 so the case can be copied and removed."""
    for base, dirs, files in os.walk(d):
        for n in dirs + files:
            p = os.path.join(base, n)
            if not os.path.islink(p):
                try:
                    os.chmod(p, 0o755)
                except OSError:
                    pass
    for base, dirs, _ in os.walk(d):
        for n in dirs:
            p = os.path.join(base, n)
            if not os.path.islink(p):
                try:
                    os.chmod(p, 0o755)
                except OSError:
                    pass


def run(cmd, timeout, env=None, cwd=None):
    """(returncode or None on timeout, stdout tail, stderr tail, seconds)."""
    t = time.time()
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL, env=env, cwd=cwd,
                         start_new_session=True)
    try:
        out, err = p.communicate(timeout=timeout)
        code = p.returncode
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, err = p.communicate()
        code = None
    return code, out.decode("utf-8", "replace"), err[-6000:].decode("utf-8", "replace"), time.time() - t


def judge(code, out, err, allowed):
    """None if fine, else a short reason."""
    if code is None:
        return "timeout"
    if "panicked at" in err or "overflowed its stack" in err:
        m = re.search(r"panicked at ([^\n]+)\n([^\n]*)", err) or re.search(r"(thread .* overflowed its stack)", err)
        return "panic: " + (" ".join(g for g in m.groups() if g) if m else err.strip().splitlines()[-1][:300])
    if code < 0:
        return f"killed by signal {-code}"
    if "internal error" in err:
        return "daemon internal error (caught panic)"
    if code not in allowed:
        return f"exit {code}: {err.strip()[-300:]}"
    if code == 1 and "Error:" not in err and not out.strip():
        return f"exit 1 without findings or an error message: {err.strip()[-300:]}"
    return None


def git(d, *args):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
    subprocess.run(["git", "-C", d, *args], env=env, check=True, capture_output=True)


def run_case(exe, i, case, a, tmp_root):
    fam, name, spec, daemon_roll = case
    d = tempfile.mkdtemp(prefix=f"c{i}-", dir=tmp_root)
    res = {"case": i, "family": fam, "name": name, "failures": [], "seconds": 0.0}
    try:
        materialize(spec, d)
        env = dict(os.environ, NO_COLOR="1")
        code, out, err, secs = run([exe, "check", "--root", d, "--format", "json"], a.timeout, env)
        res["seconds"] = secs
        why = judge(code, out, err, (0, 1))
        # a report (exit 0, or 1 for error findings) is always a JSON array
        if why is None and code in (0, 1) and not (code == 1 and "Error:" in err):
            try:
                if not isinstance(json.loads(out), list):
                    why = "check printed JSON that is not a list of findings"
            except json.JSONDecodeError:
                why = "check printed no JSON, or JSON that does not parse"
        if why:
            res["failures"].append(("check", why))
        # not cases with unreadable files or pipes: git cannot add them (as
        # any user but root), and that is not what the daemon path tests
        if a.daemon_every and daemon_roll < 1.0 / a.daemon_every and not res["failures"] and not spec.get("special"):
            fails, served = daemon_path(exe, d, spec, a, env)
            res["failures"] += fails
            res["daemon"] = served
    except Exception as e:  # a bug in the fuzzer itself, not in grounds
        res["failures"].append(("fuzzer", f"{type(e).__name__}: {e}"))
    finally:
        if res["failures"] and a.keep:
            dest = os.path.join(a.keep, f"case-{i}")
            unlock(d)
            shutil.rmtree(dest, ignore_errors=True)
            try:
                shutil.copytree(d, dest, symlinks=True, ignore=shutil.ignore_patterns(".git"))
            except shutil.Error:
                pass  # named pipes are not copied: FAILURE.txt names the case to regenerate it
            with open(os.path.join(dest, "FAILURE.txt"), "w") as fh:
                fh.write(f"seed {a.seed} case {i} ({fam}: {name})\n")
                for where, why in res["failures"]:
                    fh.write(f"{where}: {why}\n")
                fh.write(f"rerun: python3 -I scripts/fuzz_robust.py --seed {a.seed} --case {i} --family {a.family} "
                         f"--daemon-every {a.daemon_every} --timeout {a.timeout:g}\n")
                # inside this repository its .gitignore hides target/ from grounds
                fh.write(f"or:    cp -r {dest} /tmp/case && grounds check --root /tmp/case --format json\n")
        unlock(d)
        shutil.rmtree(d, ignore_errors=True)
    return res


def daemon_path(exe, d, spec, a, env):
    """The case as a git repository with an uncommitted edit, checked twice by `grounds changed`
    (the first run starts the daemon, the second goes through it), then the daemon stopped."""
    fails, served = [], False
    env = dict(env, XDG_RUNTIME_DIR=os.path.join(os.path.dirname(d), "xdg-" + os.path.basename(d)))
    os.makedirs(env["XDG_RUNTIME_DIR"], mode=0o700, exist_ok=True)
    try:
        git(d, "init", "-q")
        git(d, "add", "-A")
        git(d, "commit", "-qm", "init", "--allow-empty")
        # the edit: every Java file gets a line appended, one is truncated
        java = [p for p in glob.glob(os.path.join(d, "**", "*.java"), recursive=True) if os.path.isfile(p)
                and not os.path.islink(p) and os.access(p, os.W_OK)]
        for p in java:
            with open(p, "ab") as fh:
                fh.write(b"\nclass Edited { void m() { String s = null; s.trim(); } }\n")
        if java:
            with open(java[0], "rb") as fh:
                data = fh.read()
            with open(java[0], "wb") as fh:
                fh.write(data[:len(data) * 2 // 3])
        for attempt in range(2):
            code, out, err, _ = run([exe, "changed", "--root", d, "--json", "--all-lines"], a.timeout, env)
            why = judge(code, out, err, (0, 1, 2))
            if why:
                fails.append((f"changed#{attempt + 1}", why))
                break
            if attempt == 0:
                # wait for the daemon the first run started, 10 s in all
                deadline = time.monotonic() + 10
                while (left := deadline - time.monotonic()) > 0:
                    c, o, _, _ = run([exe, "daemon", "--status", "--root", d], max(left, 0.1), env)
                    if o.startswith("running"):
                        served = True
                        break
                    time.sleep(0.05)
    finally:
        code, out, err, _ = run([exe, "daemon", "--stop", "--root", d], 30, env)
        if code is None or "panicked" in err:
            fails.append(("daemon --stop", judge(code, out, err, (0,)) or f"exit {code}"))
        shutil.rmtree(env["XDG_RUNTIME_DIR"], ignore_errors=True)
    # served: the second `changed` found a daemon running (else it ran in-process)
    return fails, served


def grounds_exe():
    if os.environ.get("GROUNDS_BIN"):
        return os.environ["GROUNDS_BIN"]
    subprocess.run(["cargo", "build", "-q", "--release", "-p", "grounds-cli"], cwd=REPO, check=True)
    return os.path.join(os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target"), "release", "grounds")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--cases", type=int, default=3000)
    ap.add_argument("--case", type=int, help="run only this case of the seed")
    ap.add_argument("--family", default=",".join(FAMILIES))
    ap.add_argument("--timeout", type=float, default=20.0, help="seconds per grounds run")
    ap.add_argument("--daemon-every", type=int, default=25, help="send about one case in N through the daemon (0: none)")
    ap.add_argument("--keep", help="directory for failing cases (default target/fuzz-robust/seed-S)")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    a = ap.parse_args()
    fams = [f for f in a.family.split(",") if f]
    if any(f not in FAMILIES for f in fams):
        sys.exit(f"unknown family in {a.family} ({', '.join(FAMILIES)})")
    a.keep = os.path.abspath(a.keep or os.path.join(REPO, "target", "fuzz-robust", f"seed-{a.seed}"))
    if a.case is None:
        shutil.rmtree(a.keep, ignore_errors=True)
    os.makedirs(a.keep, exist_ok=True)
    exe = grounds_exe()
    seeds = load_seeds()
    ids = [a.case] if a.case is not None else list(range(1, a.cases + 1))
    tmp_root = tempfile.mkdtemp(prefix="grounds-robust-")
    t0 = time.time()
    counts = {f: {"cases": 0, "failed": 0, "daemon": 0} for f in fams}
    failures = []
    slowest = []
    try:
        with ThreadPoolExecutor(a.jobs) as ex:
            futs = [ex.submit(lambda i=i: run_case(exe, i, make_case(a.seed, i, fams, seeds), a, tmp_root)) for i in ids]
            for f in futs:
                r = f.result()
                c = counts[r["family"]]
                c["cases"] += 1
                c["daemon"] += 1 if r.get("daemon") else 0
                slowest.append((r["seconds"], r["case"], r["name"]))
                if r["failures"]:
                    c["failed"] += 1
                    failures.append(r)
                    for where, why in r["failures"]:
                        print(f"FAIL seed {a.seed} case {r['case']} [{r['family']}: {r['name']}] {where}: {why}", flush=True)
    finally:
        shutil.rmtree(tmp_root, ignore_errors=True)
    slowest.sort(reverse=True)
    report = {"seed": a.seed, "counts": counts, "seconds": round(time.time() - t0, 1),
              "slowest": [{"seconds": round(s, 2), "case": c, "name": n} for s, c, n in slowest[:10]],
              "failures": [{"case": r["case"], "family": r["family"], "name": r["name"], "failures": r["failures"]}
                           for r in failures]}
    with open(os.path.join(a.keep, "report.json"), "w") as fh:
        json.dump(report, fh, indent=1)
    print(f"seed {a.seed}: " + "; ".join(f"{f} {c['cases']} cases ({c['daemon']} also through the daemon), {c['failed']} failed"
                                         for f, c in counts.items()) + f"; {report['seconds']} s")
    print("slowest: " + ", ".join(f"case {c} {s:.1f}s ({n})" for s, c, n in slowest[:3]))
    if failures:
        print(f"{len(failures)} failing case(s) kept in {a.keep}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
