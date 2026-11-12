#!/usr/bin/env python3
"""Bytecode oracle: is grounds right about what the code refers to?

A class file records what javac decided: for every call the method chosen
(owner, name, descriptor), for every field access the field, the source line
of each, and the members of the class. A library published on Maven Central
comes as two jars, the classes and the sources they were compiled from: real
code, already compiled by its own build with its own classpath, Lombok and
annotation processors, with nothing to build here. This script fetches such
pairs, unpacks the sources as a project (no dependency jars, so grounds sees
what it sees of an unbuilt checkout) and the classes beside it, and runs the
`bytecode_oracle` example of grounds-cli, which reads the class files itself
and compares calls, constructors, fields, method reference names and declared
members with what grounds resolves.

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
CALLS = re.compile(r"found in bytecode (\d+)  agree (\d+)  other overload (\d+)  other owner (\d+)  other return (\d+)")
CATEGORY = re.compile(r"^(constructors|fields|method reference names|members): .*?compared (\d+)  agree (\d+)  disagree (\d+)  not compared (\d+)", re.M)


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


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dir", default="target/oracle")
    ap.add_argument("--bin", default="target/release/examples/bytecode_oracle")
    ap.add_argument("coordinates", nargs="+", help="group:artifact:version")
    a = ap.parse_args()
    binary = os.path.abspath(a.bin)
    totals = [0, 0, 0, 0]
    categories = {}  # name -> [compared, agree, disagree, not compared]
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
        out = subprocess.run([binary, proj, classes], capture_output=True, text=True, timeout=1800).stdout
        print(f"== {c}\n{out}", end="")
        m = CALLS.search(out)
        if m:
            n = [int(x) for x in m.groups()]
            totals = [t + x for t, x in zip(totals, [n[0], n[1], sum(n[2:]), 1])]
        for m in CATEGORY.finditer(out):
            row = categories.setdefault(m.group(1), [0, 0, 0, 0])
            for i, x in enumerate(m.groups()[1:]):
                row[i] += int(x)
    compared, agree, disagree, artifacts = totals
    print(f"\ntotal: {artifacts} artifact(s), {compared} certain calls compared, {agree} agree, {disagree} disagree")
    for name, (compared, agree, disagree, not_compared) in categories.items():
        print(f"total {name}: {compared} compared, {agree} agree, {disagree} disagree, {not_compared} not compared")


if __name__ == "__main__":
    sys.exit(main())
