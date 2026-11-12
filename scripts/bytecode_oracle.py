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
and compares with what grounds resolves: calls, constructors, fields, method
reference names and declared members. The class files are read, never run.

A gate. Every disagreement is keyed by artifact, category, source file, line,
subject (the name; for members the class and the member) and the two
answers. The baseline (scripts/bytecode_oracle.baseline.json) lists the
known ones, one per line, and for each artifact and category how many
agreed. The exit status is 1 when

  - a disagreement occurs that the baseline does not list (NEW),
  - a baseline entry no longer occurs (STALE: a fix deletes its entries),
  - a category of an artifact agrees on more than 2% fewer than the baseline
    records (DROP: grounds became unsure of what it used to know; agreements
    are counted, not comparisons, so that removing answers that were wrong
    is not a drop),
  - the example fails on an artifact, or an artifact is not in the baseline,
  - more than a quarter of the artifacts could not be fetched.

An artifact that cannot be fetched is skipped, not failed: Maven Central
answers 429 to quick downloads. Downloads are kept in --dir and never
fetched again, so a run that stops resumes where it stopped; between two
downloads it pauses --pause seconds.

  python3 -I scripts/bytecode_oracle.py --list corpus/oracle.tsv
      [--dir target/oracle] [--out target/oracle-report] [--bin PATH]
      [--baseline scripts/bytecode_oracle.baseline.json]
      [--update-baseline | --prune-baseline] [--pause 10]
  python3 -I scripts/bytecode_oracle.py org.mockito:mockito-core:5.8.0 ...

--update-baseline rewrites the baseline from this run: the entries and
counts of the artifacts that ran are replaced, a note on an entry that still
occurs is kept. --prune-baseline only deletes the entries that no longer
occur. Mark an entry that is not a grounds error (javac's own naming) with a
"note". To add an artifact: a line in corpus/oracle.tsv (group, artifact,
version, why it is there), a run with --update-baseline, and a look at each
disagreement it records: a grounds error stays, a javac convention is either
allowed in the example (narrowly) or noted, a comparison bug is fixed.
Without --bin it builds the example with cargo. Writes OUT/report.json (totals,
new, stale, drops, skipped) and OUT/<artifact>.json (the example's output).
"""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import time
import zipfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASE = "https://repo1.maven.org/maven2"
CATEGORIES = ["calls", "constructors", "fields", "method references", "members"]
# what makes a disagreement the same one, in the order a baseline line has them
KEY = ["artifact", "category", "kind", "file", "line", "subject", "member", "grounds", "javac"]
# a category may compare this much less than the baseline records
TOLERANCE = 0.02


def read_list(path):
    """group:artifact:version of each line of a list (tab-separated group,
    artifact, version, note; # starts a comment)."""
    out = []
    with open(path) as fh:
        for line in fh:
            if line.startswith("#") or not line.strip():
                continue
            g, a, v = line.rstrip("\n").split("\t")[:3]
            out.append(f"{g}:{a}:{v}")
    return out


def fetch(url, dest):
    """True when it downloaded, False when it was on disk; RuntimeError when
    it cannot be had (404 at once, anything else after the retries)."""
    if os.path.exists(dest):
        return False
    for wait in (0, 10, 30, 90, 180):
        time.sleep(wait)
        r = subprocess.run(["curl", "-sSL", "-m", "300", "-w", "%{http_code}", "-o", dest + ".part", url],
                           capture_output=True, text=True)
        code = r.stdout.strip()
        if r.returncode == 0 and code == "200":
            os.rename(dest + ".part", dest)
            return True
        if code == "404":
            break
    if os.path.exists(dest + ".part"):
        os.remove(dest + ".part")
    raise RuntimeError(f"cannot fetch {url} (last status {code or r.returncode})")


def unpack(jar, dest, suffix):
    os.makedirs(dest)
    with zipfile.ZipFile(jar) as z:
        for n in z.namelist():
            # no path may leave dest; multi-release copies would be second copies
            if n.endswith(suffix) and ".." not in n and not n.startswith(("/", "META-INF/")):
                z.extract(n, dest)


def prepare(d):
    """Unpack the jars in d into proj/ and classes/ once: into .part
    directories renamed at the end, so an interrupted unpack is redone."""
    proj, classes = os.path.join(d, "proj"), os.path.join(d, "classes")
    if os.path.isdir(proj):
        return proj, classes
    for p in (proj + ".part", classes + ".part"):
        shutil.rmtree(p, ignore_errors=True)
    unpack(os.path.join(d, "sources.jar"), os.path.join(proj + ".part", "src", "main", "java"), ".java")
    if not os.path.isdir(classes):
        unpack(os.path.join(d, "classes.jar"), classes + ".part", ".class")
        os.rename(classes + ".part", classes)
    # the release the classes were compiled for, from a class file's version
    with open(next(pathlib.Path(classes).rglob("*.class")), "rb") as f:
        release = int.from_bytes(f.read(8)[6:8], "big") - 44
    with open(os.path.join(proj + ".part", "grounds.toml"), "w") as f:
        f.write(f"java_release = {min(release, 21)}\n[classpath]\nscan_caches = false\n")
    subprocess.run(["git", "init", "-q", proj + ".part"], check=True)
    os.rename(proj + ".part", proj)
    return proj, classes


def key(e):
    return tuple(e.get(k, "") for k in KEY)


def load_baseline(path):
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        return {"counts": {}, "known": []}
    with open(path) as fh:
        b = json.load(fh)
    return {"counts": b.get("counts", {}), "known": b.get("known", [])}


def write_baseline(path, counts, known):
    """One artifact's counts per line, one entry per line, sorted: a fix or a
    regression is a line removed or added."""
    lines = ["{", ' "counts": {']
    arts = sorted(counts)
    for i, a in enumerate(arts):
        c = {k: counts[a][k] for k in CATEGORIES if k in counts[a]}
        lines.append(f"  {json.dumps(a)}: {json.dumps(c)}" + ("," if i < len(arts) - 1 else ""))
    lines += [" },", ' "known": [']
    known = sorted(known, key=key)
    for i, e in enumerate(known):
        o = {k: e[k] for k in KEY if e.get(k, "") != ""}
        if e.get("note"):
            o["note"] = e["note"]
        lines.append("  " + json.dumps(o, ensure_ascii=False) + ("," if i < len(known) - 1 else ""))
    lines += [" ]", "}"]
    with open(path, "w") as fh:
        fh.write("\n".join(lines) + "\n")


def show(e):
    member = f" {e['member']}" if e.get("member") else ""
    note = f"  [{e['note']}]" if e.get("note") else ""
    return (f"{e['artifact']} {e['category']} {e['kind']}  {e['file']}:{e['line']} {e['subject']}{member}: "
            f"grounds {e['grounds']}, javac {e['javac']}{note}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", help="a list of artifacts, as corpus/oracle.tsv")
    ap.add_argument("--dir", default="target/oracle", help="downloads and unpacked jars, kept between runs")
    ap.add_argument("--out", default="target/oracle-report", help="report.json and each artifact's output")
    ap.add_argument("--bin", help="the bytecode_oracle example; built with cargo when not given")
    ap.add_argument("--baseline", default=os.path.join(REPO, "scripts", "bytecode_oracle.baseline.json"))
    ap.add_argument("--update-baseline", action="store_true", help="rewrite the baseline from this run")
    ap.add_argument("--prune-baseline", action="store_true", help="only delete baseline entries that no longer occur")
    ap.add_argument("--pause", type=float, default=10, help="seconds between two downloads")
    ap.add_argument("coordinates", nargs="*", help="group:artifact:version")
    a = ap.parse_args()
    coords = (read_list(a.list) if a.list else []) + a.coordinates
    if not coords:
        ap.error("no artifacts: give --list or coordinates")
    whole_list = bool(a.list) and not a.coordinates
    if a.bin:
        binary = os.path.abspath(a.bin)
    else:
        subprocess.run(["cargo", "build", "-q", "--release", "-p", "grounds-cli", "--example", "bytecode_oracle"],
                       cwd=REPO, check=True)
        target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
        binary = os.path.join(target, "release", "examples", "bytecode_oracle")
    out_dir = os.path.abspath(a.out)
    os.makedirs(out_dir, exist_ok=True)

    base = load_baseline(a.baseline)
    known_by_art = {}
    for e in base["known"]:
        known_by_art.setdefault(e["artifact"], []).append(e)

    totals = {c: {"compared": 0, "agree": 0, "disagree": 0, "not_compared": 0} for c in CATEGORIES}
    counts, found, skipped, failed, ran = {}, {}, [], [], []
    downloaded = False
    for c in coords:
        g, art, v = c.split(":")
        d = os.path.abspath(os.path.join(a.dir, f"{art}-{v}"))
        os.makedirs(d, exist_ok=True)
        url = f"{BASE}/{g.replace('.', '/')}/{art}/{v}/{art}-{v}"
        try:
            for suffix, name in ((".jar", "classes.jar"), ("-sources.jar", "sources.jar")):
                dest = os.path.join(d, name)
                if downloaded and not os.path.exists(dest):
                    time.sleep(a.pause)
                downloaded = fetch(url + suffix, dest) or downloaded
        except RuntimeError as e:
            print(f"== {c}: SKIPPED, {e}", flush=True)
            skipped.append(c)
            if not os.listdir(d):
                os.rmdir(d)
            continue
        proj, classes = prepare(d)
        try:
            p = subprocess.run([binary, "--json", proj, classes], capture_output=True, text=True, timeout=1800)
            r = json.loads(p.stdout) if p.returncode == 0 else None
        except (subprocess.TimeoutExpired, json.JSONDecodeError) as e:
            p, r = None, None
        if r is None:
            why = p.stderr.strip()[-2000:] if p else "timed out or unreadable output"
            print(f"== {c}: FAILED\n{why}", flush=True)
            failed.append(c)
            continue
        with open(os.path.join(out_dir, f"{art}-{v}.json"), "w") as fh:
            json.dump(r, fh, indent=1, sort_keys=True)
        ran.append(c)
        counts[c] = {k: r["categories"][k]["agree"] for k in CATEGORIES}
        for k in CATEGORIES:
            for n, x in r["categories"][k].items():
                totals[k][n] += x
        found[c] = {key(e): e for e in ({**x, "artifact": c} for x in r["disagreements"])}
        print(f"== {c}: " + ", ".join(f"{k} {r['categories'][k]['compared']}/{r['categories'][k]['disagree']}" for k in CATEGORIES)
              + "  (compared/disagree)", flush=True)

    # judge against the baseline
    new, stale, drops, missing = [], [], [], []
    for c in ran:
        old = {key(e): e for e in known_by_art.get(c, [])}
        new += [e for k, e in sorted(found[c].items()) if k not in old]
        stale += [e for k, e in sorted(old.items()) if k not in found[c]]
        if c not in base["counts"]:
            missing.append(c)
            continue
        for k in CATEGORIES:
            was = base["counts"][c].get(k, 0)
            if counts[c][k] < was * (1 - TOLERANCE):
                drops.append(f"{c} {k}: agree {counts[c][k]}, the baseline records {was}")
    # artifacts the baseline knows that the list no longer has
    gone = sorted((set(base["counts"]) | set(known_by_art)) - set(coords)) if whole_list else []
    stale += [e for c in gone for e in known_by_art.get(c, [])]

    print()
    for k in CATEGORIES:
        t = totals[k]
        print(f"total {k}: {t['compared']} compared, {t['agree']} agree, {t['disagree']} disagree, {t['not_compared']} not compared")
    print(f"artifacts: {len(ran)} compared, {len(skipped)} skipped, {len(failed)} failed")

    if a.update_baseline or a.prune_baseline:
        keep_counts = {c: n for c, n in base["counts"].items() if c not in gone}
        keep_known = [e for e in base["known"] if e["artifact"] not in gone]
        if a.update_baseline:
            notes = {key(e): e["note"] for e in base["known"] if e.get("note")}
            keep_counts = {**{c: n for c, n in keep_counts.items() if c not in ran}, **counts}
            keep_known = [e for e in keep_known if e["artifact"] not in ran]
            for c in ran:
                for k, e in found[c].items():
                    keep_known.append({**e, "note": notes[k]} if k in notes else e)
        else:
            gone_keys = {key(e) for e in stale}
            keep_known = [e for e in keep_known if key(e) not in gone_keys]
        write_baseline(a.baseline, keep_counts, keep_known)
        print(f"baseline written: {len(keep_known)} known disagreement(s), counts of {len(keep_counts)} artifact(s)")
        # what was written is no longer a failure; a prune accepts nothing new
        stale, gone = [], []
        if a.update_baseline:
            new, drops, missing = [], [], []

    for e in new:
        print(f"NEW    {show(e)}")
    for e in stale:
        print(f"STALE  {show(e)}  (no longer occurs: delete it from the baseline, or --prune-baseline)")
    for x in drops:
        print(f"DROP   {x}")
    for c in missing:
        print(f"NOT IN BASELINE  {c} (run with --update-baseline)")
    for c in gone:
        print(f"NOT IN LIST  {c} is in the baseline (--update-baseline or --prune-baseline removes it)")
    too_many_skipped = len(skipped) * 4 > len(coords)
    if too_many_skipped:
        print(f"SKIPPED {len(skipped)} of {len(coords)} artifacts: more than a quarter")

    report = {
        "artifacts": len(coords), "compared": ran, "skipped": skipped, "failed": failed, "totals": totals,
        "new": new, "stale": stale, "drops": drops, "not_in_baseline": missing, "not_in_list": gone,
    }
    with open(os.path.join(out_dir, "report.json"), "w") as fh:
        json.dump(report, fh, indent=1, sort_keys=True)
    bad = new or stale or drops or missing or gone or failed or too_many_skipped
    print("gate: " + ("FAIL" if bad else "pass"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
