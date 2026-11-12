#!/usr/bin/env python3
"""Bytecode oracle: is grounds right about which method a call reaches?

A class file records, for every call, the method javac chose (owner, name,
descriptor) and the line of the call. A library published on Maven Central
comes as two jars, the classes and the sources they were compiled from: real
code, already compiled by its own build with its own classpath, Lombok and
annotation processors, with nothing to build here. This script fetches such
pairs, unpacks the sources as a project (no dependency jars, so grounds sees
what it sees of an unbuilt checkout), lists the invoke instructions of the
classes with `javap`, and runs the `bytecode_oracle` example of grounds-cli,
which compares every call grounds is certain of with the method javac chose.

A probe that measures; it gates nothing. The class files are read, never run.

  cargo build --release -p grounds-cli --example bytecode_oracle
  python3 -I scripts/bytecode_oracle.py [--dir target/oracle] [--bin target/release/examples/bytecode_oracle] \\
      org.apache.commons:commons-collections4:4.4 org.mockito:mockito-core:5.8.0 ...
"""
import argparse
import os
import pathlib
import re
import subprocess
import sys
import time
import zipfile

BASE = "https://repo1.maven.org/maven2"
INVOKE = re.compile(r"^\s*(\d+): invoke(?:virtual|static|special|interface)\s+#\d+(?:,\s*\d+)?\s+// (?:Interface)?Method (.+)$")
HEADER = re.compile(r"^(?:\w+ )*(?:class|interface) ([\w.$]+)")
LINE = re.compile(r"^\s*line (\d+): (\d+)$")


def fetch(url, dest):
    if os.path.exists(dest):
        return
    for wait in (0, 5, 20, 60):
        time.sleep(wait)
        r = subprocess.run(["curl", "-fsSL", "-m", "180", "-o", dest + ".part", url])
        if r.returncode == 0:
            os.rename(dest + ".part", dest)
            return
    raise RuntimeError(f"cannot fetch {url}")


def unpack(jar, dest, suffix):
    os.makedirs(dest)
    with zipfile.ZipFile(jar) as z:
        for n in z.namelist():
            # no path may leave dest; multi-release copies would be second copies
            if n.endswith(suffix) and ".." not in n and not n.startswith(("/", "META-INF/")):
                z.extract(n, dest)


def invokes(classes, out):
    """Every invoke instruction of every class as: source file, line, owner, name, descriptor."""
    files = [str(p) for p in pathlib.Path(classes).rglob("*.class") if p.name not in ("module-info.class", "package-info.class")]
    env = {k: v for k, v in os.environ.items() if k != "JAVA_TOOL_OPTIONS"}
    rows = 0
    with open(out, "w") as w:
        for i in range(0, len(files), 400):
            text = subprocess.run(["javap", "-c", "-l", "-p"] + files[i:i + 400], capture_output=True, text=True, env=env).stdout
            cls = src = None
            code, lines = [], []  # (pc, owner, name, descriptor), (pc, line)

            def flush():
                nonlocal code, lines, rows
                if cls and src and code and lines:
                    lines.sort()
                    pkg = cls.rsplit("/", 1)[0] + "/" if "/" in cls else ""
                    for pc, owner, name, desc in code:
                        at = [l for lpc, l in lines if lpc <= pc]
                        if at:
                            w.write(f"{pkg}{src}\t{at[-1]}\t{owner}\t{name}\t{desc}\n")
                            rows += 1
                code, lines = [], []

            for line in text.splitlines():
                if line.startswith("Compiled from "):
                    flush()
                    src, cls = line.split('"')[1], None
                elif cls is None:
                    m = HEADER.match(line)
                    if m:
                        cls = m.group(1).replace(".", "/")
                elif line.strip() == "Code:":
                    flush()
                elif m := INVOKE.match(line):
                    target, desc = m.group(2).rsplit(":", 1)
                    # javap leaves the owner out for a method of the class itself
                    owner, name = target.rsplit(".", 1) if "." in target and not target.startswith('"<') else (cls, target)
                    code.append((int(m.group(1)), owner.strip('"'), name.strip('"'), desc))
                elif m := LINE.match(line):
                    lines.append((int(m.group(2)), int(m.group(1))))
            flush()
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", default="target/oracle")
    ap.add_argument("--bin", default="target/release/examples/bytecode_oracle")
    ap.add_argument("coordinates", nargs="+", help="group:artifact:version")
    a = ap.parse_args()
    binary = os.path.abspath(a.bin)
    totals = [0, 0, 0, 0]
    for c in a.coordinates:
        g, art, v = c.split(":")
        d = os.path.abspath(os.path.join(a.dir, f"{art}-{v}"))
        os.makedirs(d, exist_ok=True)
        url = f"{BASE}/{g.replace('.', '/')}/{art}/{v}/{art}-{v}"
        try:
            fetch(url + ".jar", os.path.join(d, "classes.jar"))
            fetch(url + "-sources.jar", os.path.join(d, "sources.jar"))
        except RuntimeError as e:
            print(f"== {c}: {e}")
            continue
        proj, classes = os.path.join(d, "proj"), os.path.join(d, "classes")
        if not os.path.isdir(proj):
            unpack(os.path.join(d, "sources.jar"), os.path.join(proj, "src", "main", "java"), ".java")
            unpack(os.path.join(d, "classes.jar"), classes, ".class")
            # the release the classes were compiled for, from a class file's version
            with open(next(pathlib.Path(classes).rglob("*.class")), "rb") as f:
                release = int.from_bytes(f.read(8)[6:8], "big") - 44
            with open(os.path.join(proj, "grounds.toml"), "w") as f:
                f.write(f"java_release = {min(release, 21)}\n[classpath]\nscan_caches = false\n")
            subprocess.run(["git", "init", "-q", proj], check=True)
        tsv = os.path.join(d, "bytecode.tsv")
        if not os.path.exists(tsv):
            invokes(classes, tsv)
        out = subprocess.run([binary, proj, tsv], capture_output=True, text=True, timeout=1800).stdout
        print(f"== {c}\n{out}", end="")
        m = re.search(r"found in bytecode (\d+)  agree (\d+)  other overload (\d+)  other owner (\d+)  other return (\d+)", out)
        if m:
            n = [int(x) for x in m.groups()]
            totals = [t + x for t, x in zip(totals, [n[0], n[1], sum(n[2:]), 1])]
    compared, agree, disagree, artifacts = totals
    print(f"\ntotal: {artifacts} artifact(s), {compared} certain calls compared, {agree} agree, {disagree} disagree")


if __name__ == "__main__":
    sys.exit(main())
