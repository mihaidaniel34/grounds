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
            is a statistic, not a failure. "Other files call" is a guess
            from `.name(` anywhere in another file, so not every one of these
            breaks the build; the kinds below are chosen so that each does.

With --kinds, edits that certainly break compilation of another file, up to
--per-kind of each. Every one needs a call site or use in another file
(main or test sources) that is known to reach the edited declaration, found
on the source with comments and string literals blanked:

  - the other file names the edited type C by a single-type import of its
    qualified name, or is in C's package and has no single-type import of
    another C; it declares no type named C; C's qualified name is declared
    by one file only, and C's simple name has more than two characters
    (so it is no type parameter);
  - the receiver is C itself (a static member), or a variable every
    declaration of which in that file has type C, that is no lambda
    parameter there, and that cannot be an inherited field instead (it is a
    field of the file's top-level class, or that class's supertypes are all
    known and declare no such field);
  - the member is the only one of its name in C and in all of C's
    supertypes, all of which are project types or JDK types whose members
    are listed here (Object, Serializable, Comparable, Runnable, Closeable,
    the common exceptions); not in a record (implicit accessors), not an
    Object or Enum method, and in a class with Lombok annotations not a name
    Lombok may generate (getX, setX, isX, withX, builder, of, ...).

With that, each kind breaks because:

  rename-method      `x.m(..)` names a method C no longer has.
  remove-method      the same, with the declaration deleted.
  add-parameter      an `int` parameter appended (not to varargs): C's only
                     `m` now takes one more argument than the call passes.
  change-return      the return type becomes `Void` (the body becomes a
                     throw), and the caller uses the result as the old type:
                     declares a variable of exactly the old type (not Object,
                     not a type variable) from it, calls a method on it that
                     is not one of Object's, or negates it (a boolean).
                     `Void` is final and implements nothing, so it converts
                     to no other type.
  make-private       `public` becomes `private` on a non-abstract method of
                     a class: another top-level class cannot access it.
  delete-class       a public top-level type deleted, used by another file
                     through a single-type import (importing a type that does
                     not exist is an error), or in the same package as
                     `new C(`, `C.x`, a declaration of type C, `extends C` or
                     `implements C`, by a file without on-demand imports, C's
                     simple name not being one of java.lang's. C's package
                     must hold another file, so it is not a patched copy of a
                     library class that a jar would supply again.
  rename-class       the same type renamed, with its file and its own
                     references to itself.
  interface-remove-method
                     an abstract method of an interface deleted while another
                     file calls it through a variable of the interface type.
  add-abstract-method
                     a new abstract method in an interface (or abstract
                     class) that a non-abstract class in another file
                     implements (extends) directly, or that an anonymous
                     class `new I() {` in another file implements: that
                     class does not implement the new method.
  rename-field       a public field or constant (one declarator) renamed
                     while another file reads `C.NAME` or `x.NAME`, C having
                     no nested type of that name.

The scan cannot see everything the compiler sees (a nested class inherited
from a library supertype named like C, a library class supplying C's
qualified name, a file that the build does not compile), so
`--confirm REPORT --build-dir COPY` replays a sample of the mutations of a
report on a separate checkout at the same commit (compiled there, not in
DIR, so that grounds never sees build outputs) and records whether the
compiler rejects it: with `--build CMD` the project's build (confirmed when
CMD fails), with `--javac` the JDK's javac on the files expected to break,
before and after the mutation, without dependency jars or annotation
processing (confirmed when a new error appears in an expected file).

Deterministic: the seed and the project's files fix the mutations. The
checkout is reset after each mutation; uncommitted changes are refused.
With --daemon every mutation is also asked through the daemon, which has
followed every edit so far, and the two answers must agree.

  python3 -I scripts/ci_mutations.py DIR [--seed 1] [--safe 12] [--breaking 6]
      [--kinds all|k1,k2] [--per-kind 4] [--daemon]
      [--grounds target/release/grounds] [--out report.json]
  python3 -I scripts/ci_mutations.py --confirm report.json --build-dir COPY (--build CMD | --javac)
      [--per-kind 2] [--timeout 1800] [--out confirmed.json]
  python3 -I scripts/ci_mutations.py --summary report.json

Exit status 1 if a safe edit got a report, or the daemon and a fresh run
disagreed.
"""

import argparse
import json
import os
import random
import re
import subprocess
import sys
import time

COMPILE_RULES = {"unresolved-method", "unresolved-constructor", "unresolved-field", "unresolved-static-import",
                 "override-matches-nothing", "unimplemented-abstract-method", "incompatible-assignment",
                 "removed-class-still-used", "breaks-other-files"}
CLASS_RE = re.compile(r"^(?:public\s+)?(?:(?:final|abstract)\s+)*class\s+\w+[^;{]*\{\s*$")
PACKAGE_RE = re.compile(r"^package\s+([\w.]+)\s*;")
METHOD_RE = re.compile(r"^\s+public\s+(?:(?:static|final|synchronized)\s+)*[\w.<>\[\], ?]+\s+(\w+)\s*\([^)]*\)\s*(?:throws [\w., ]+)?\s*\{\s*$")

KINDS = ["rename-method", "remove-method", "add-parameter", "change-return", "make-private", "delete-class",
         "rename-class", "interface-remove-method", "add-abstract-method", "rename-field"]


def git(d, *args, check=True):
    return subprocess.run(["git", "-C", d, *args], capture_output=True, text=True, check=check).stdout


def sources(d):
    files = git(d, "ls-files", "--", "*.java").splitlines()
    return sorted(f for f in files if "/src/main/java/" in "/" + f and "/generated" not in f)


def read(d, f):
    # newline="": keep CRLF line ends, or rewriting the file would change every line
    with open(os.path.join(d, f), encoding="utf-8", errors="replace", newline="") as fh:
        return fh.read().split("\n")


def write(d, f, lines):
    if sum(x.endswith("\r") for x in lines) * 2 > len(lines):  # a CRLF file: inserted lines too
        lines = [x if x.endswith("\r") or i == len(lines) - 1 else x + "\r" for i, x in enumerate(lines)]
    with open(os.path.join(d, f), "w", encoding="utf-8", newline="") as fh:
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
        if subprocess.run(["git", "-C", d, "check-ignore", "-q", new]).returncode == 0:
            return None  # a tracked file under an ignored directory: a new file there would not be committed
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


# ---------------------------------------------------------------------------
# A scan of Java sources good enough to pick edits that certainly break:
# comments and literals blanked, braces matched, top-level types and their
# members. Anything it cannot read with certainty is left out.

JAVA_LANG = set("""AbstractMethodError Appendable ArithmeticException ArrayIndexOutOfBoundsException
ArrayStoreException AssertionError AutoCloseable Boolean BootstrapMethodError Byte CharSequence Character
Class ClassCastException ClassCircularityError ClassFormatError ClassLoader ClassNotFoundException ClassValue
CloneNotSupportedException Cloneable Comparable Compiler Deprecated Double Enum EnumConstantNotPresentException
Error Exception ExceptionInInitializerError Float FunctionalInterface IllegalAccessError IllegalAccessException
IllegalArgumentException IllegalCallerException IllegalMonitorStateException IllegalStateException
IllegalThreadStateException IncompatibleClassChangeError IndexOutOfBoundsException InheritableThreadLocal
InstantiationError InstantiationException Integer InternalError InterruptedException Iterable
LayerInstantiationException LinkageError Long MatchException Math Module ModuleLayer NegativeArraySizeException
NoClassDefFoundError NoSuchFieldError NoSuchFieldException NoSuchMethodError NoSuchMethodException
NullPointerException Number NumberFormatException Object OutOfMemoryError Override Package Process
ProcessBuilder ProcessHandle Readable Record ReflectiveOperationException Runnable Runtime RuntimeException
RuntimePermission SafeVarargs ScopedValue SecurityException SecurityManager Short StackOverflowError
StackTraceElement StackWalker StrictMath String StringBuffer StringBuilder StringIndexOutOfBoundsException
StringTemplate SuppressWarnings System Thread ThreadDeath ThreadGroup ThreadLocal Throwable
TypeNotPresentException UnknownError UnsatisfiedLinkError UnsupportedClassVersionError
UnsupportedOperationException VerifyError VirtualMachineError Void WrongThreadException""".split())
OBJECT_METHODS = {"getClass", "hashCode", "equals", "clone", "toString", "notify", "notifyAll", "wait", "finalize"}
ENUM_METHODS = {"name", "ordinal", "compareTo", "valueOf", "values", "getDeclaringClass", "describeConstable"}
THROWABLE_METHODS = {"getMessage", "getLocalizedMessage", "getCause", "initCause", "printStackTrace",
                     "getStackTrace", "setStackTrace", "fillInStackTrace", "addSuppressed", "getSuppressed"}
# JDK supertypes whose public members are known: name -> (packages it may come from, method names)
JDK_SUPERS = {"Object": (("java.lang",), set()), "Serializable": (("java.io",), set()),
              "Cloneable": (("java.lang",), set()), "Comparable": (("java.lang",), {"compareTo"}),
              "Runnable": (("java.lang",), {"run"}), "AutoCloseable": (("java.lang",), {"close"}),
              "Closeable": (("java.io",), {"close"})}
for _e in ("Throwable", "Exception", "RuntimeException", "Error", "IllegalStateException",
           "IllegalArgumentException", "UnsupportedOperationException"):
    JDK_SUPERS[_e] = (("java.lang",), THROWABLE_METHODS)
LOMBOK_PREFIXES = ("get", "set", "is", "with", "builder", "toBuilder", "of", "canEqual")
MODIFIERS = ("public", "protected", "private", "static", "final", "abstract", "synchronized", "native",
             "default", "strictfp", "transient", "volatile", "sealed", "non-sealed")
KEYWORDS = {"return", "throw", "new", "case", "else", "yield", "assert", "do", "package", "import", "goto",
            "break", "continue", "instanceof", "extends", "implements", "throws", "super", "this"}
MODS = r"(?P<mods>(?:(?:" + "|".join(MODIFIERS) + r")\s+)*)"
TYPE = r"[\w$.]+(?:\s*<[^{};()]*>)?(?:\s*\[\s*\])*"
NOT_MOD = r"(?!(?:" + "|".join(MODIFIERS) + r")\b)"
MEMBER_METHOD_RE = re.compile(r"^\s*" + MODS + r"(?P<tp><[^{};]*?>\s*)?" + NOT_MOD + r"(?P<ret>" + TYPE + r")\s+"
                              r"(?P<name>[A-Za-z_$][\w$]*)\s*\((?P<params>.*)\)\s*(?:\[\s*\]\s*)*"
                              r"(?:throws\s+[\w$.,\s<>]+)?\s*$", re.S)
MEMBER_FIELD_RE = re.compile(r"^\s*" + MODS + NOT_MOD + r"(?P<type>" + TYPE + r")\s+(?P<name>[A-Za-z_$][\w$]*)\s*"
                             r"(?:\[\s*\]\s*)*(?:=.*)?$", re.S)
TYPE_DECL_RE = re.compile(r"(?<![\w$.])(class|interface|enum|record|@\s*interface)\s+([A-Za-z_$][\w$]*)")
IMPORT_RE = re.compile(r"^\s*import\s+(static\s+)?([\w$.]+?)(\.\*)?\s*;", re.M)


def blank_literals(src):
    """`src` with comments and the insides of string, text block and char
    literals replaced by spaces; same length, same line breaks."""
    out = list(src)
    i, n = 0, len(src)

    def blank(a, b):
        for j in range(a, b):
            if out[j] != "\n":
                out[j] = " "
    while i < n:
        c = src[i]
        if c == "/" and src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif c == "/" and src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            blank(i, j)
            i = j
        elif src.startswith('"""', i):
            j = i + 3
            while j < n and not (src.startswith('"""', j) and (j - i) >= 3):
                j += 2 if src[j] == "\\" else 1
            blank(i + 3, min(j, n))
            i = j + 3
        elif c in "\"'":
            j = i + 1
            while j < n and src[j] != c and src[j] != "\n":
                j += 2 if src[j] == "\\" else 1
            blank(i + 1, min(j, n))
            i = j + 1
        else:
            i += 1
    return "".join(out)


def blank_annotations(s):
    """`s` with annotations (and their balanced arguments) replaced by spaces."""
    out = list(s)
    for m in re.finditer(r"@(?!\s*interface\b)\s*[\w$.]+", s):
        a, j = m.start(), m.end()
        k = j
        while k < len(s) and s[k] in " \t\r\n":
            k += 1
        if k < len(s) and s[k] == "(":
            depth = 0
            while k < len(s):
                depth += {"(": 1, ")": -1}.get(s[k], 0)
                k += 1
                if depth == 0:
                    break
            j = k
        for x in range(a, j):
            if out[x] != "\n":
                out[x] = " "
    return "".join(out)


def strip_generics(t):
    out, depth = [], 0
    for ch in t:
        if ch == "<":
            depth += 1
        elif ch == ">":
            depth -= 1
        elif depth == 0:
            out.append(ch)
    return "".join(out).strip()


def split_top(s, sep=","):
    parts, depth, cur = [], 0, []
    for ch in s:
        if ch in "<(":
            depth += 1
        elif ch in ">)":
            depth -= 1
        if ch == sep and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    parts.append("".join(cur))
    return [p.strip() for p in parts if p.strip()]


class JFile:
    def __init__(self, path, src):
        self.path, self.src = path, src
        self.code = blank_literals(src)
        self.lines_at = None
        m = re.search(r"^\s*package\s+([\w$.]+)\s*;", self.code, re.M)
        self.pkg = m.group(1) if m else ""
        self.single, self.ondemand, self.static_imports = {}, [], False
        for m in IMPORT_RE.finditer(self.code):
            if m.group(1):
                self.static_imports = True
                continue
            if m.group(3):
                self.ondemand.append(m.group(2))
            else:
                self.single[m.group(2).rsplit(".", 1)[-1]] = m.group(2)
        self.match = {}
        stack = []
        self.depth = [0] * (len(self.code) + 1)
        dep = 0
        for i, ch in enumerate(self.code):
            self.depth[i] = dep
            if ch == "{":
                stack.append(i)
                dep += 1
            elif ch == "}":
                if stack:
                    self.match[stack.pop()] = i
                dep = max(0, dep - 1)
        self.depth[len(self.code)] = dep
        self.declared_types = set()
        self.types = []
        for m in TYPE_DECL_RE.finditer(self.code):
            self.declared_types.add(m.group(2))
            if self.depth[m.start()] == 0:
                t = self.parse_type(m)
                if t:
                    self.types.append(t)

    def line(self, pos):
        return self.code.count("\n", 0, pos) + 1

    def stmt_start(self, pos, depth):
        """Start of the statement or member around `pos` at brace depth `depth`."""
        j = pos - 1
        while j >= 0:
            ch = self.code[j]
            if ch in ";{}" and (self.depth[j] == depth or (ch == "}" and self.depth[j] == depth + 1)
                                or (ch == "{" and self.depth[j] == depth - 1)):
                return j + 1
            j -= 1
        return 0

    def parse_type(self, m):
        kind = m.group(1).replace(" ", "")
        d = self.depth[m.start()]
        open_ = self.code.find("{", m.end())
        if open_ < 0 or open_ not in self.match:
            return None
        header = self.code[m.end():open_]
        pre = blank_annotations(self.code[self.stmt_start(m.start(), d):m.start()])
        mods = set(pre.split())
        t = {"kind": kind, "name": m.group(2), "file": self, "fqn": (self.pkg + "." if self.pkg else "") + m.group(2),
             "mods": mods, "open": open_, "close": self.match[open_], "decl": m.start(), "extends": [],
             "implements": [], "tparams": [], "methods": [], "fields": [], "nested": set()}
        h = header
        if h.lstrip().startswith("<"):
            s = h.index("<")
            dep, k = 0, s
            while k < len(h):
                dep += {"<": 1, ">": -1}.get(h[k], 0)
                k += 1
                if dep == 0:
                    break
            t["tparams"] = [p.split()[0] for p in split_top(h[s + 1:k - 1]) if p.split()]
            h = h[k:]
        h = strip_generics(h)
        if kind == "record" or "(" in h:
            t["kind"] = "record"
            return t
        ext = re.search(r"\bextends\s+(.*?)(?=\bimplements\b|\bpermits\b|$)", h, re.S)
        imp = re.search(r"\bimplements\s+(.*?)(?=\bpermits\b|$)", h, re.S)
        if ext:
            t["extends"] = [x.strip() for x in ext.group(1).split(",") if x.strip()]
        if imp:
            t["implements"] = [x.strip() for x in imp.group(1).split(",") if x.strip()]
        self.parse_members(t)
        return t

    def parse_members(self, t):
        code, d = self.code, self.depth[t["open"]] + 1
        i, end = t["open"] + 1, t["close"]
        if t["kind"] == "enum":
            # members start after the constants' `;`
            j = i
            while j < end and not (code[j] == ";" and self.depth[j] == d):
                j += 1
            if j >= end:
                return
            i = j + 1
        start = i
        while i < end:
            ch = code[i]
            if ch == ";" and self.depth[i] == d:
                self.member(t, start, i, ";")
                start = i + 1
            elif ch == "{" and self.depth[i] == d:
                close = self.match.get(i, end)
                self.member(t, start, i, "{", close)
                chunk = code[start:i]
                if "=" in strip_generics(chunk) and not re.search(r"\)\s*(?:throws[^=]*)?$", chunk.strip()):
                    # a field initializer with braces: the member ends at its `;`
                    j = close + 1
                    while j < end and not (code[j] == ";" and self.depth[j] == d):
                        j += 1
                    i = j
                else:
                    i = close
                start = i + 1
            i += 1

    def member(self, t, a, b, term, close=None):
        raw = self.code[a:b]
        chunk = blank_annotations(raw)
        if not chunk.strip():
            return
        tm = TYPE_DECL_RE.search(chunk)
        if tm:
            t["nested"].add(tm.group(2))
            return
        # the declaration starts at its first annotation or modifier
        lead = len(raw) - len(raw.lstrip())
        first = a + lead
        mods_iface = t["kind"] in ("interface", "@interface")
        mm = MEMBER_METHOD_RE.match(chunk) if "=" not in chunk.split("(")[0] else None
        if mm and mm.group("ret") not in KEYWORDS and mm.group("name") not in KEYWORDS:
            mods = set(mm.group("mods").split())
            has_body = term == "{"
            abstract = "abstract" in mods or (mods_iface and not has_body)
            public = "public" in mods or (mods_iface and "private" not in mods)
            t["methods"].append({
                "name": mm.group("name"), "mods": mods, "public": public, "abstract": abstract,
                "static": "static" in mods, "ret": re.sub(r"\s+", "", mm.group("ret")),
                "ret_span": (a + mm.start("ret"), a + mm.end("ret")),
                "name_span": (a + mm.start("name"), a + mm.end("name")),
                "params": mm.group("params"), "params_span": (a + mm.start("params"), a + mm.end("params")),
                "tparams": [p.split()[0] for p in split_top((mm.group("tp") or "").strip()[1:-1]) if p.split()],
                "body": (b, close) if has_body else None,
                "span": (first, (close + 1) if has_body else b + 1),
                "line": self.line(a + mm.start("name"))})
            return
        if "(" in chunk.split("=")[0]:
            return  # a constructor, initializer or something not read
        fm = MEMBER_FIELD_RE.match(chunk if term == ";" else chunk + " ")
        if not fm or fm.group("type") in KEYWORDS:
            return
        before_eq = chunk.split("=")[0]
        if "," in strip_generics(before_eq):
            return  # several declarators
        mods = set(fm.group("mods").split())
        public = "public" in mods or (mods_iface and "private" not in mods)
        t["fields"].append({"name": fm.group("name"), "mods": mods, "public": public,
                            "static": "static" in mods or mods_iface,
                            "name_span": (a + fm.start("name"), a + fm.end("name")),
                            "line": self.line(a + fm.start("name"))})


class Project:
    def __init__(self, d):
        self.d = d
        tracked = git(d, "ls-files", "--", "*.java").splitlines()
        self.files = {}
        for f in tracked:
            p = "/" + f
            if ("/src/main/java/" not in p and "/src/test/java/" not in p) or "/generated" in p \
                    or "/resources/" in p or "archetype" in p:
                continue
            try:
                with open(os.path.join(d, f), encoding="utf-8", errors="replace", newline="") as fh:
                    self.files[f] = JFile(f, fh.read())
            except OSError:
                continue
        self.by_fqn = {}
        self.pkg_files = {}
        self.words, self.dotted = {}, {}  # identifier -> files with it; with `.` before it
        self.recv_cache = {}
        for jf in self.files.values():
            for w in set(re.findall(r"[A-Za-z_$][\w$]*", jf.code)):
                self.words.setdefault(w, []).append(jf)
            for w in set(re.findall(r"\.\s*([A-Za-z_$][\w$]*)", jf.code)):
                self.dotted.setdefault(w, set()).add(jf.path)
            self.pkg_files.setdefault(jf.pkg, []).append(jf)
            for t in jf.types:
                self.by_fqn.setdefault(t["fqn"], []).append(t)

    def unique(self, fqn):
        ts = self.by_fqn.get(fqn, [])
        return ts[0] if len(ts) == 1 else None

    def resolve(self, jf, name):
        """A supertype named in `jf`: ("project", type), ("jdk", methods) or None (unknown)."""
        name = strip_generics(name)
        if "." in name:
            t = self.unique(name)
            return ("project", t) if t else None
        if name in jf.declared_types:
            return None  # a nested type of the file: not read
        if name in jf.single:
            fqn = jf.single[name]
            t = self.unique(fqn)
            if t:
                return ("project", t)
            j = JDK_SUPERS.get(name)
            return ("jdk", j[1]) if j and fqn.rsplit(".", 1)[0] in j[0] else None
        if self.by_fqn.get(f"{jf.pkg}.{name}" if jf.pkg else name):
            t = self.unique(f"{jf.pkg}.{name}" if jf.pkg else name)
            return ("project", t) if t else None
        hits = [self.unique(f"{p}.{name}") for p in jf.ondemand if self.by_fqn.get(f"{p}.{name}")]
        if hits:
            return ("project", hits[0]) if len(hits) == 1 and hits[0] else None
        j = JDK_SUPERS.get(name)
        if j and ("java.lang" in j[0] or any(p in j[0] for p in jf.ondemand)):
            return ("jdk", j[1])
        return None

    def declares(self, t, name, what, seen=None):
        """Whether `t` or a supertype declares a method (what="method") or field
        named `name`, beyond `t`'s own one: None when a supertype is unknown."""
        seen = seen or set()
        if t["fqn"] in seen:
            return False
        seen.add(t["fqn"])
        supers = t["extends"] + t["implements"]
        if t["kind"] == "enum" and what == "method" and name in ENUM_METHODS:
            return True
        for s in supers:
            r = self.resolve(t["file"], s)
            if r is None:
                return None
            if r[0] == "jdk":
                if what == "method" and name in r[1]:
                    return True
                continue
            st = r[1]
            if st["kind"] == "record":
                return None
            own = st["methods"] if what == "method" else st["fields"]
            if any(x["name"] == name for x in own):
                return True
            up = self.declares(st, name, what, seen)
            if up is None or up:
                return up
        return False

    def visible(self, jf, t):
        """Whether `t`'s simple name in `jf` certainly means `t`."""
        if jf.path == t["file"].path or t["name"] in jf.declared_types:
            return False
        if t["name"] in jf.single:
            return jf.single[t["name"]] == t["fqn"]
        return jf.pkg == t["file"].pkg

    def receivers(self, jf, t):
        """Variables in `jf` that certainly have type `t`."""
        key = (jf.path, t["fqn"])
        if key not in self.recv_cache:
            self.recv_cache[key] = self._receivers(jf, t)
        return self.recv_cache[key]

    def _receivers(self, jf, t):
        c = re.escape(t["name"])
        names = set(re.findall(r"(?<![\w$.])" + c + r"(?:\s*<[^;(){}=]*?>)?\s+([A-Za-z_$][\w$]*)\s*(?=[=;,):])",
                               jf.code))
        out = set()
        top = jf.types[0] if jf.types else None
        for v in names - KEYWORDS:
            ve = re.escape(v)
            decl_types = re.findall(r"(?<![\w$.])([\w$.]+)(?:\s*<[^;(){}=]*?>)?(?:\s*\[\s*\])*\s+" + ve +
                                    r"\s*(?=[=;,):])", jf.code)
            if any(x != t["name"] for x in decl_types if x != "final" and x not in KEYWORDS):
                continue
            if re.search(r"(?<![\w$.])" + ve + r"\s*->", jf.code) or \
                    re.search(r"\(\s*(?:[\w$]+\s*,\s*)*" + ve + r"(?:\s*,\s*[\w$]+)*\s*\)\s*->", jf.code):
                continue
            if top is None:
                continue
            own_field = any(f["name"] == v for f in top["fields"])
            if not own_field and self.declares(top, v, "field") is not False:
                continue
            out.add(v)
        return out

    def uses(self, t, member, static, kind="method"):
        """(file, line) of uses of `t`'s member in other files: calls for methods,
        reads for fields, with the receiver's span."""
        out = []
        tail = r"\s*\(" if kind == "method" else r"\b(?!\s*\()"
        with_member = self.dotted.get(member, set())
        for jf in self.words.get(t["name"], []):
            if jf.path not in with_member or not self.visible(jf, t):
                continue
            recv = set(self.receivers(jf, t))
            if static:
                recv.add(t["name"])
            for v in sorted(recv):
                for m in re.finditer(r"(?<![\w$.])" + re.escape(v) + r"\s*\.\s*" + re.escape(member) + tail,
                                     jf.code):
                    out.append((jf, m))
        return out


def member_ok(p, t, name, what):
    """The only member of its name in `t` and its supertypes, and not one that
    Object, Enum or Lombok may supply."""
    if t["kind"] == "record" or len(t["name"]) <= 2 or name.startswith("zz"):
        return False
    if what == "method" and (name in OBJECT_METHODS or name == "main"):
        return False
    own = t["methods"] if what == "method" else t["fields"]
    if sum(1 for x in own if x["name"] == name) != 1:
        return False
    if what == "field" and name in t["nested"]:
        return False
    if re.search(r"^\s*import\s+lombok\b|@lombok\.", t["file"].code, re.M):
        if what == "method" and (name.startswith(LOMBOK_PREFIXES) or name in OBJECT_METHODS):
            return False
    return p.declares(t, name, what) is False


def certain_candidates(p, kind, main_files):
    """Every (description, apply(k) -> changed paths, expected sites) for `kind`."""
    out = []
    for path in main_files:
        jf = p.files.get(path)
        if not jf:
            continue
        for t in jf.types:
            if p.unique(t["fqn"]) is not t or t["kind"] in ("record", "@interface"):
                continue
            if kind in ("rename-method", "remove-method", "add-parameter", "change-return", "make-private",
                        "interface-remove-method"):
                want_iface = kind == "interface-remove-method"
                if (t["kind"] == "interface") != want_iface:
                    continue
                for m in t["methods"]:
                    if not m["public"] or not member_ok(p, t, m["name"], "method"):
                        continue
                    if want_iface and (not m["abstract"] or m["static"]):
                        continue
                    if kind == "make-private" and (m["abstract"] or m["body"] is None):
                        continue
                    if kind == "add-parameter" and "..." in m["params"]:
                        continue
                    if kind == "change-return" and (m["ret"] in ("void", "Void", "Object", "java.lang.Object")
                                                    or m["body"] is None):
                        continue
                    sites = p.uses(t, m["name"], m["static"])
                    if kind == "change-return":
                        sites = [s for s in sites if uses_result(s[0], s[1], m, t)]
                    if not sites:
                        continue
                    out.append((kind, t, m, sites))
            elif kind == "rename-field":
                for fd in t["fields"]:
                    if not fd["public"] or not member_ok(p, t, fd["name"], "field"):
                        continue
                    sites = p.uses(t, fd["name"], fd["static"], "field")
                    if sites:
                        out.append((kind, t, fd, sites))
            elif kind in ("delete-class", "rename-class"):
                if "public" not in t["mods"] or len(jf.types) != 1 or len(t["name"]) <= 2 \
                        or t["name"] in JAVA_LANG or len(p.pkg_files.get(jf.pkg, [])) < 2 \
                        or os.path.basename(path) != t["name"] + ".java":
                    continue
                sites = class_uses(p, t)
                if sites:
                    out.append((kind, t, None, sites))
            elif kind == "add-abstract-method":
                if not (t["kind"] == "interface" or (t["kind"] == "class" and "abstract" in t["mods"])):
                    continue
                if len(t["name"]) <= 2:
                    continue
                sites = implementors(p, t)
                if sites:
                    out.append((kind, t, None, sites))
    return out


def uses_result(jf, m, meth, t):
    """Whether the call matched by `m` uses its result as the old return type."""
    code = jf.code
    j, depth = m.end() - 1, 0
    while j < len(code):
        depth += {"(": 1, ")": -1}.get(code[j], 0)
        j += 1
        if depth == 0:
            break
    after = code[j:j + 200]
    a = re.match(r"\s*\.\s*([A-Za-z_$][\w$]*)\s*[(]", after)
    if a and a.group(1) not in OBJECT_METHODS:
        return True
    before = code[max(0, m.start() - 300):m.start()]
    ret = meth["ret"]
    tvars = set(t["tparams"]) | set(meth["tparams"])
    if strip_generics(ret) in tvars or ret in ("var",):
        return False
    b = re.search(r"(?<![\w$.])([\w$.]+(?:\s*<[^;(){}=]*?>)?(?:\s*\[\s*\])*)\s+[A-Za-z_$][\w$]*\s*=\s*$", before)
    if b and re.sub(r"\s+", "", b.group(1)) == ret and b.group(1) not in KEYWORDS:
        return True
    if ret == "boolean" and re.search(r"(?<![!=<>])!\s*$", before):
        return True
    return False


def class_uses(p, t):
    """Uses of type `t` in other files that stop compiling without it."""
    out = []
    c = re.escape(t["name"])
    for jf in p.words.get(t["name"], []):
        if jf.path == t["file"].path or t["name"] in jf.declared_types:
            continue
        imp = re.search(r"^\s*import\s+" + re.escape(t["fqn"]) + r"\s*;", jf.code, re.M)
        if imp:
            out.append((jf, imp))
            continue
        if jf.pkg != t["file"].pkg or t["name"] in jf.single or jf.ondemand or jf.static_imports:
            continue
        m = re.search(r"(?<![\w$.])(?:new\s+" + c + r"\s*[(<]|" + c + r"\s*\.\s*[\w$]|" + c +
                      r"(?:\s*<[^;(){}=]*?>)?\s+[A-Za-z_$][\w$]*\s*[=;,):]|(?:extends|implements)\s+" + c + r"\b)",
                      jf.code)
        if m:
            out.append((jf, m))
    return out


def implementors(p, t):
    """Non-abstract classes in other files that directly implement (extend) `t`,
    and anonymous classes of an interface `t`."""
    out = []
    iface = t["kind"] == "interface"
    for jf in p.words.get(t["name"], []):
        if not p.visible(jf, t):
            continue
        for m in TYPE_DECL_RE.finditer(jf.code):
            if m.group(1) not in ("class", "enum", "record"):
                continue
            pre = blank_annotations(jf.code[jf.stmt_start(m.start(), jf.depth[m.start()]):m.start()])
            if "abstract" in pre.split():
                continue
            open_ = jf.code.find("{", m.end())
            if open_ < 0:
                continue
            h = strip_generics(jf.code[m.end():open_])
            if m.group(1) == "record":
                h = re.sub(r"\(.*?\)", " ", h, flags=re.S)
            if iface:
                imp = re.search(r"\bimplements\s+(.*)$", h, re.S)
                names = [x.strip() for x in imp.group(1).split(",")] if imp else []
            else:
                ext = re.search(r"\bextends\s+([\w$.]+)", h)
                names = [ext.group(1)] if ext and m.group(1) == "class" else []
            if t["name"] in names:
                out.append((jf, m))
        if iface:
            for m in re.finditer(r"(?<![\w$.])new\s+" + re.escape(t["name"]) + r"\s*(?:<[^;(){}]*?>)?\s*\(\s*\)\s*\{",
                                 jf.code):
                out.append((jf, m))
    return out


def apply_certain(d, p, cand, k):
    """Edit the working tree for `cand`; (description, changed paths, removed paths)."""
    kind, t, member, sites = cand
    jf = t["file"]
    src = jf.src
    path = jf.path

    def save(text, where=path):
        with open(os.path.join(d, where), "w", encoding="utf-8", newline="") as fh:
            fh.write(text)
    if kind == "rename-method" or kind == "rename-field":
        a, b = member["name_span"]
        save(src[:a] + member["name"] + "ZzRenamed" + src[b:])
    elif kind in ("remove-method", "interface-remove-method"):
        a, b = member["span"]
        save(src[:a] + src[b:])
    elif kind == "add-parameter":
        a, b = member["params_span"]
        inner = src[a:b]
        new = (inner.rstrip() + ", int zzMut" + str(k)) if inner.strip() else "int zzMut" + str(k)
        save(src[:a] + new + src[b:])
    elif kind == "change-return":
        ra, rb = member["ret_span"]
        ba, bb = member["body"]
        save(src[:ra] + "Void" + src[rb:ba] + "{ throw new UnsupportedOperationException(); }" + src[bb + 1:])
    elif kind == "make-private":
        a, b = member["span"]
        head = src[a:member["name_span"][0]]
        code_head = jf.code[a:member["name_span"][0]]
        mm = re.search(r"\bpublic\b", code_head)
        if not mm:
            return None
        save(src[:a] + head[:mm.start()] + "private" + head[mm.end():] + src[member["name_span"][0]:])
    elif kind == "delete-class":
        os.remove(os.path.join(d, path))
        return f"{kind} {path} {t['fqn']}", [path], [path]
    elif kind == "rename-class":
        new_name = t["name"] + "ZzRenamed"
        out, last = [], 0
        for m in re.finditer(r"(?<![\w$])" + re.escape(t["name"]) + r"(?![\w$])", jf.code):
            out.append(src[last:m.start()] + new_name)
            last = m.end()
        out.append(src[last:])
        new_path = os.path.join(os.path.dirname(path), new_name + ".java")
        git(d, "mv", path, new_path)
        save("".join(out), new_path)
        return f"{kind} {path} {t['fqn']}", [new_path], [path]
    elif kind == "add-abstract-method":
        o = t["open"]
        decl = f"void zzMutAbstract{k}();" if t["kind"] == "interface" else f"public abstract void zzMutAbstract{k}();"
        save(src[:o + 1] + "\n    " + decl + src[o + 1:])
        return f"{kind} {path}:{jf.line(o)} {t['fqn']}", [path], []
    name = member["name"]
    return f"{kind} {path}:{member['line']} {t['name']}.{name}", [path], []


def changed(grounds, d, daemon=False):
    cmd = [grounds, "changed", "--root", d, "--base", "HEAD~1", "--format", "json", "--exit-zero", "--max",
           "100000", "--min-severity", "info", "--color", "never"]
    if not daemon:
        cmd.append("--no-daemon")
    p = subprocess.run(cmd, capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if p.returncode != 0:
        raise RuntimeError(f"grounds changed failed ({p.returncode}): {p.stderr[-2000:]}")
    return json.loads(p.stdout)


def normal(r):
    return {k: sorted(json.dumps(x, sort_keys=True) for x in v) if isinstance(v, list) else v for k, v in r.items()}


def difference(a, b):
    """What answer `a` has and `b` lacks, as lines (both normalized)."""
    out = []
    for k, v in a.items():
        if isinstance(v, list):
            for x in v:
                if x not in (b.get(k) or []):
                    f = json.loads(x)
                    f = f.get("finding", f) if isinstance(f, dict) else f
                    out.append(f"{k}: " + (where(f) + ": " + f.get("message", "") if isinstance(f, dict) and "rule" in f
                                           else str(f)))
        elif v != b.get(k):
            out.append(f"{k}: {v}")
    return out[:20]


def commit(d, msg):
    git(d, "add", "-A", "--", "*.java")
    git(d, "-c", "user.name=mutation", "-c", "user.email=mutation@localhost", "-c", "commit.gpgsign=false",
        "commit", "-q", "--no-verify", "-m", msg)


def summary(path):
    with open(path) as fh:
        st = json.load(fh)["stats"]
    print(f"mutations: safe edits {st['safe']} ({st['safe-reported']} reported), "
          f"breaking renames {st['breaking']} ({st['breaking-reported']} reported)")
    for kind in KINDS:
        if st.get(kind):
            print(f"  {kind}: {st[kind]} ({st.get(kind + '-reported', 0)} reported)")
    if st.get("daemon-differs") is not None:
        print(f"  daemon differs from a fresh run on {st['daemon-differs']}")
    return 0


def where(f):
    return f"{f['path']}:{f['line']} {f['rule']}"


def judge_break(r, touched, expected):
    pending = r.get("pending", [])
    findings = [f for f in r.get("findings", []) if f["rule"] in COMPILE_RULES]
    breaks = [b["finding"] for b in r.get("breaks", [])]
    other = pending + [f for f in findings if f["path"] not in touched]
    inside = [f for f in findings if f["path"] in touched]
    exp = set(expected)
    return {"reported": bool(other or breaks),
            "reported-in-changed-file-only": bool(inside) and not (other or breaks),
            "rules": sorted({f["rule"] for f in other + inside + breaks}),
            "reports": [where(f) + ": " + f["message"] for f in (other + inside + breaks)][:20],
            "expected-hit": any(f"{f['path']}:{f['line']}" in exp for f in other)}


def run_certain(a, d, grounds, head, stats, results, k0):
    p = Project(d)
    main_files = [f for f in p.files if "/src/main/java/" in "/" + f]
    kinds = KINDS if a.kinds == "all" else [x for x in a.kinds.split(",") if x]
    unknown = [x for x in kinds if x not in KINDS]
    if unknown:
        sys.exit(f"unknown kinds: {', '.join(unknown)} (known: {', '.join(KINDS)})")
    plan = []
    for kind in kinds:
        cands = certain_candidates(p, kind, sorted(main_files))
        stats[kind + "-candidates"] = len(cands)
        rng = random.Random(f"{a.seed}:{os.path.basename(d)}:{kind}")
        # spread over files: one candidate per declaring file first
        rng.shuffle(cands)
        seen, first, rest = set(), [], []
        for c in cands:
            key = c[1]["file"].path
            (rest if key in seen else first).append(c)
            seen.add(key)
        plan += [(kind, c) for c in (first + rest)[:a.per_kind]]
    k = k0
    for kind, cand in plan:
        k += 1
        applied = apply_certain(d, p, cand, k)
        if not applied:
            git(d, "reset", "-q", "--hard", head)
            continue
        desc, touched, removed = applied
        sites = sorted({f"{jf.path}:{jf.line(m.start())}" for jf, m in cand[3]})
        commit(d, desc)
        patch = git(d, "diff", "--binary", "-M", "HEAD~1", "HEAD")
        entry = {"mutation": desc, "kind": kind, "expected": sites[:20], "expected-count": len(sites),
                 "expected-files": len({s.rsplit(':', 1)[0] for s in sites}), "patch": patch}
        try:
            t0 = time.monotonic()
            r = changed(grounds, d)
            entry["seconds"] = round(time.monotonic() - t0, 2)
            if a.daemon:
                w = changed(grounds, d, daemon=True)
                if normal(w) != normal(r):
                    entry["daemon-differs"] = True
                    entry["daemon-only"] = difference(normal(w), normal(r))
                    entry["fresh-only"] = difference(normal(r), normal(w))
                    stats["daemon-differs"] += 1
                    print(f"DAEMON DIFFERS from a fresh run after {desc}")
                    for x in entry["daemon-only"][:5]:
                        print(f"    daemon only: {x}")
                    for x in entry["fresh-only"][:5]:
                        print(f"    fresh only:  {x}")
        finally:
            git(d, "reset", "-q", "--hard", head)
            git(d, "clean", "-q", "-f", "--", "*ZzMut*.java", "*ZzRenamed*.java")
        entry.update(judge_break(r, set(touched) | set(removed), sites))
        stats[kind] = stats.get(kind, 0) + 1
        stats[kind + "-reported"] = stats.get(kind + "-reported", 0) + entry["reported"]
        results.append(entry)
        print(f"  {'REPORTED' if entry['reported'] else 'silent  '} {desc} ({len(sites)} expected)", flush=True)
    return k


JAVAC_ERROR_RE = re.compile(r"^(.+?\.java):(\d+): error: (.*)$", re.M)


def javac_errors(d, files, roots, timeout):
    """javac's errors for `files` (sources found on `roots`), as {(path, line, message)}; None on timeout."""
    import tempfile
    with tempfile.TemporaryDirectory() as out:
        cmd = ["javac", "-proc:none", "-implicit:none", "-nowarn", "-Xlint:none", "-encoding", "UTF-8",
               "-Xmaxerrs", "1000000", "-XDshould-stop.ifError=FLOW", "-d", out,
               "-sourcepath", os.pathsep.join(roots)] + files
        try:
            p = subprocess.run(cmd, cwd=d, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            return None
    return {(os.path.normpath(m.group(1)), int(m.group(2)), m.group(3).strip())
            for m in JAVAC_ERROR_RE.finditer(p.stdout + p.stderr)}


def confirm(a):
    """Replay a sample of a report's mutations on a separate checkout and compile.

    With --build CMD the project's own build runs, and a mutation is
    confirmed when CMD fails. With --javac the JDK's javac compiles the files
    expected to break and the mutated files (other sources found on the
    module source roots; no annotation processing, no dependency jars), before
    and after the mutation: a mutation is confirmed when javac reports an
    error in an expected file that it did not report before. Errors from
    missing jars are there both times; an expected file whose break they hide
    (a call inside a lambda passed to a library method, say) leaves the
    mutation unconfirmed rather than refuted; `expected-clean-before` says
    whether javac had no error at all in the expected files before the edit,
    in which case an unconfirmed mutation is refuted.
    """
    with open(a.confirm) as fh:
        rep = json.load(fh)
    d = os.path.abspath(a.build_dir)
    if git(d, "rev-parse", "HEAD").strip() != rep.get("head"):
        sys.exit(f"{d}: not at the report's commit {rep.get('head')}")
    if git(d, "status", "--porcelain", "--untracked-files=no").strip():
        sys.exit(f"{d}: uncommitted changes to tracked files")

    def build():
        t0 = time.monotonic()
        try:
            p = subprocess.run(a.build, shell=True, cwd=d, capture_output=True, text=True, timeout=a.timeout,
                               stdin=subprocess.DEVNULL)
            return p.returncode, p.stdout + p.stderr, time.monotonic() - t0
        except subprocess.TimeoutExpired:
            return None, "timeout", time.monotonic() - t0
    roots = []
    if a.javac:
        found = set()
        for f in git(d, "ls-files", "--", "*.java").splitlines():
            for marker in ("src/main/java/", "src/test/java/"):
                i = ("/" + f).find("/" + marker)
                if i >= 0:
                    found.add(f[:i + len(marker) - 1] if i > 0 else marker.rstrip("/"))
        roots = sorted(found)
    else:
        code, out, secs = build()
        if code != 0:
            print(out[-3000:])
            sys.exit(f"{d}: the unmutated checkout does not build ({code}); nothing to confirm")
        print(f"{rep['project']}: baseline builds in {secs:.0f} s", flush=True)
    rng = random.Random(f"{a.seed}:confirm:{rep['project']}")
    by_kind = {}
    for i, m in enumerate(rep["mutations"]):
        if m.get("patch"):
            by_kind.setdefault(m["kind"], []).append(i)
    picked = []
    for kind in sorted(by_kind):
        ids = by_kind[kind]
        rng.shuffle(ids)
        picked += ids[:a.per_kind]
    out_rows = []

    def reset():
        git(d, "reset", "-q", "--hard", "HEAD")
        git(d, "clean", "-q", "-f", "--", "*ZzMut*.java", "*ZzRenamed*.java")
    try:
        for i in sorted(picked):
            m = rep["mutations"][i]
            expected_files = sorted({e.rsplit(":", 1)[0] for e in m.get("expected", [])})
            row = {"index": i, "mutation": m["mutation"], "kind": m["kind"]}
            t0 = time.monotonic()
            if a.javac:
                changed_files = [x for x in re.findall(r"^\+\+\+ b/(.+)$", m["patch"], re.M)]
                files = sorted(set(expected_files[:8]))
                before = javac_errors(d, files + [f for f in changed_files if os.path.exists(os.path.join(d, f))],
                                      roots, a.timeout)
            p = subprocess.run(["git", "-C", d, "apply", "--index", "-"], input=m["patch"], text=True,
                               capture_output=True)
            if p.returncode != 0:  # a CRLF file: line ends may differ from the checkout's
                p = subprocess.run(["git", "-C", d, "apply", "--index", "--ignore-whitespace", "-"],
                                   input=m["patch"], text=True, capture_output=True)
            if p.returncode != 0:
                row["result"] = "patch does not apply"
                out_rows.append(row)
                reset()
                continue
            if a.javac:
                after = javac_errors(d, files + [f for f in changed_files if os.path.exists(os.path.join(d, f))],
                                     roots, a.timeout)
                if before is None or after is None:
                    row["result"] = "timeout"
                else:
                    new = sorted(after - before)
                    in_expected = [e for e in new if e[0] in files]
                    exp_lines = {(e.rsplit(":", 1)[0], int(e.rsplit(":", 1)[1])) for e in m.get("expected", [])}
                    row["result"] = "failed" if in_expected else "not confirmed"
                    # an expected file javac compiled without error before and after: the edit did not break it
                    row["expected-clean-before"] = not any(e[0] in files for e in before)
                    row["at-expected-line"] = any((e[0], e[1]) in exp_lines for e in in_expected)
                    row["new-errors"] = [f"{e[0]}:{e[1]}: {e[2]}" for e in new][:10]
                    row["files-compiled"] = files
            else:
                code, log, secs = build()
                names = [os.path.basename(f) for f in expected_files]
                row["names-expected-file"] = any(n in log for n in names)
                row["result"] = "failed" if code not in (0, None) else ("timeout" if code is None else "compiled")
                if row["result"] != "failed" or not row["names-expected-file"]:
                    row["log"] = log[-4000:]
            row["seconds"] = round(time.monotonic() - t0, 1)
            out_rows.append(row)
            print(f"  {row['result']:13} {m['mutation']}", flush=True)
            reset()
    finally:
        reset()
    if a.out:
        with open(a.out, "w") as fh:
            json.dump({"project": rep["project"], "head": rep["head"],
                       "compiler": "javac" if a.javac else a.build, "results": out_rows}, fh, indent=1)
    failed = sum(r.get("result") == "failed" for r in out_rows)
    print(f"{rep['project']}: {failed} of {len(out_rows)} mutations confirmed to break compilation")
    return 0


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--summary":
        return summary(sys.argv[2])
    ap = argparse.ArgumentParser()
    ap.add_argument("dir", nargs="?")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--safe", type=int, default=12)
    ap.add_argument("--breaking", type=int, default=6)
    ap.add_argument("--kinds", default="", help="certain breaking edits: 'all' or a comma-separated list")
    ap.add_argument("--per-kind", type=int, default=4)
    ap.add_argument("--daemon", action="store_true", help="also ask the daemon and compare")
    ap.add_argument("--grounds", default=os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                                                      "target", "release", "grounds"))
    ap.add_argument("--out")
    ap.add_argument("--confirm", metavar="REPORT")
    ap.add_argument("--build-dir")
    ap.add_argument("--build")
    ap.add_argument("--javac", action="store_true", help="confirm with the JDK's javac instead of a build")
    ap.add_argument("--timeout", type=float, default=1800)
    a = ap.parse_args()
    if a.confirm:
        if not a.build_dir or bool(a.build) == a.javac:
            ap.error("--confirm needs --build-dir and one of --build CMD or --javac")
        if "--per-kind" not in sys.argv:
            a.per_kind = 2
        return confirm(a)
    if not a.dir:
        ap.error("DIR is required")
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
    if a.daemon:
        stats["daemon-differs"] = 0
        subprocess.run([grounds, "daemon", "--root", d, "--stop"], capture_output=True)
        p = subprocess.run([grounds, "changed", "--root", d, "--format", "json", "--exit-zero"],
                           capture_output=True, text=True, stdin=subprocess.DEVNULL)  # the daemon loads the project
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
            w = None
            try:
                r = changed(grounds, d)
                if a.daemon:
                    w = changed(grounds, d, daemon=True)
            finally:
                git(d, "reset", "-q", "--hard", head)
                git(d, "clean", "-q", "-f", "--", "*ZzMut*.java")
            pending = r.get("pending", [])
            findings = r.get("findings", [])
            stats[kind] += 1
            entry = {"mutation": desc, "kind": kind}
            if w is not None and normal(w) != normal(r):
                stats["daemon-differs"] += 1
                entry["daemon-differs"] = True
                entry["daemon-only"] = difference(normal(w), normal(r))
                entry["fresh-only"] = difference(normal(r), normal(w))
                print(f"DAEMON DIFFERS from a fresh run after {desc}")
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
                entry["reported"] = [f"{f['path']}:{f['line']} {f['rule']}" for f in bad]
                entry["messages"] = [f["message"] for f in bad][:20]
            else:
                hit = bool(pending) or any(f["rule"] in COMPILE_RULES for f in findings)
                stats["breaking-reported"] += hit
                entry["reported"] = hit
            results.append(entry)
        if a.kinds:
            run_certain(a, d, grounds, head, stats, results, k)
    finally:
        git(d, "reset", "-q", "--hard", head)
        if a.daemon:
            subprocess.run([grounds, "daemon", "--root", d, "--stop"], capture_output=True)
    print(f"{os.path.basename(d)}: safe {stats['safe']} ({stats['safe-reported']} reported), "
          f"breaking {stats['breaking']} ({stats['breaking-reported']} reported)"
          + "".join(f", {x} {stats[x]} ({stats.get(x + '-reported', 0)} reported)" for x in KINDS if stats.get(x))
          + (f", daemon differs on {stats['daemon-differs']}" if a.daemon else ""))
    if a.out:
        with open(a.out, "w") as fh:
            json.dump({"project": os.path.basename(d), "head": head, "stats": stats, "mutations": results}, fh,
                      indent=1)
    return 1 if false_reports or stats.get("daemon-differs") else 0


if __name__ == "__main__":
    sys.exit(main())
