#!/usr/bin/env python3
"""Finding oracle: are grounds' findings true, as far as class files can say?

For a library published as a classes jar and the sources it was compiled
from, the `finding_oracle` example of grounds-cli runs the rules over the
sources as `grounds check` does and checks each finding of a rule with an
oracle against the class files: confirmed, refuted, or not decidable (with
the reason). Three oracles (the example's header says what each reads):
compiled (a compile-break claim on a file that compiled is false),
unreferenced (a private member the class files of its nest never refer to)
and declaration (a rule's condition recomputed from access flags,
descriptors and supertypes). The class files are read, never run.

It measures; it is no gate. Per rule it prints the findings, how many were
confirmed, refuted and not decidable, then every refuted finding on one line
(artifact, file:line, rule, what the class file shows), and the share of all
findings that an oracle decided.

  python3 -I scripts/finding_oracle.py --list corpus/oracle.tsv
      [--dir target/oracle] [--work target/finding-oracle/work]
      [--out target/finding-oracle] [--bin PATH]
  python3 -I scripts/finding_oracle.py --dir DIR org.mockito:mockito-core:5.8.0 ...

The artifacts are those scripts/bytecode_oracle.py downloads and unpacks
into --dir (`<artifact>-<version>/proj` and `classes`); run it first for an
artifact missing there. --dir is only read: each project is copied to --work
(without its `.grounds` cache) and analyzed there, since a run writes the
stub cache into the project. Without --bin it builds the example with cargo.
Writes OUT/report.json (per rule and per artifact) and OUT/<artifact>.json
(the example's output).
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
COUNTS = ["findings", "confirmed", "refuted", "undecided"]


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


def copy_project(src, dest):
    """A fresh copy of the project, without the stub cache a run left."""
    shutil.rmtree(dest, ignore_errors=True)
    shutil.copytree(src, dest, ignore=shutil.ignore_patterns(".grounds"), symlinks=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", help="a list of artifacts, as corpus/oracle.tsv")
    ap.add_argument("--dir", default="target/oracle", help="unpacked artifacts, as scripts/bytecode_oracle.py leaves them")
    ap.add_argument("--work", default="target/finding-oracle/work", help="copies of the projects the rules run on")
    ap.add_argument("--out", default="target/finding-oracle", help="report.json and each artifact's output")
    ap.add_argument("--bin", help="the finding_oracle example; built with cargo when not given")
    ap.add_argument("coordinates", nargs="*", help="group:artifact:version")
    a = ap.parse_args()
    coords = (read_list(a.list) if a.list else []) + a.coordinates
    if not coords:
        ap.error("no artifacts: give --list or coordinates")
    if a.bin:
        binary = os.path.abspath(a.bin)
    else:
        subprocess.run(["cargo", "build", "-q", "--release", "-p", "grounds-cli", "--example", "finding_oracle"],
                       cwd=REPO, check=True)
        target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
        binary = os.path.join(target, "release", "examples", "finding_oracle")
    out_dir, work = os.path.abspath(a.out), os.path.abspath(a.work)
    os.makedirs(out_dir, exist_ok=True)

    rules, arts, refuted, skipped, failed = {}, {}, [], [], []
    start = time.monotonic()
    for c in coords:
        _, art, v = c.split(":")
        name = f"{art}-{v}"
        d = os.path.abspath(os.path.join(a.dir, name))
        proj, classes = os.path.join(d, "proj"), os.path.join(d, "classes")
        if not (os.path.isdir(proj) and os.path.isdir(classes)):
            print(f"== {c}: SKIPPED, not unpacked in {a.dir} (scripts/bytecode_oracle.py does that)", flush=True)
            skipped.append(c)
            continue
        copy = os.path.join(work, name)
        copy_project(proj, copy)
        t = time.monotonic()
        try:
            p = subprocess.run([binary, "--json", copy, classes], capture_output=True, text=True, timeout=1800)
            r = json.loads(p.stdout) if p.returncode == 0 else None
        except (subprocess.TimeoutExpired, json.JSONDecodeError):
            p, r = None, None
        if r is None:
            why = p.stderr.strip()[-2000:] if p else "timed out or unreadable output"
            print(f"== {c}: FAILED\n{why}", flush=True)
            failed.append(c)
            continue
        secs = time.monotonic() - t
        with open(os.path.join(out_dir, f"{name}.json"), "w") as fh:
            json.dump(r, fh, indent=1, sort_keys=True)
        decided = 0
        for rule, x in r["rules"].items():
            s = rules.setdefault(rule, {"oracle": x["oracle"], "undecided_why": {}, "exempt": {},
                                        **{k: 0 for k in COUNTS}})
            s["findings"] += x["findings"]
            s["confirmed"] += x["confirmed"]
            s["refuted"] += x["refuted"]
            decided += x["confirmed"] + x["refuted"]
            for why, n in x["undecided"].items():
                s["undecided"] += n
                s["undecided_why"][why] = s["undecided_why"].get(why, 0) + n
            for why, n in x["exempt"].items():
                s["exempt"][why] = s["exempt"].get(why, 0) + n
        refuted += [{**e, "artifact": name} for e in r["answers"] if e["verdict"] == "refuted"]
        arts[name] = {"findings": r["findings"], "decided": decided, "seconds": round(secs, 1)}
        print(f"== {c}: findings {r['findings']}, decided {decided}, refuted "
              f"{sum(1 for e in r['answers'] if e['verdict'] == 'refuted')}  ({secs:.1f} s)", flush=True)

    print()
    print(f"{'rule':44} {'oracle':12} {'findings':>8} {'confirmed':>9} {'refuted':>7} {'not decidable':>13}")
    for rule, s in sorted(rules.items(), key=lambda x: (x[1]["oracle"] is None, x[0])):
        if s["oracle"] is None:
            continue
        print(f"{rule:44} {s['oracle']:12} {s['findings']:8} {s['confirmed']:9} {s['refuted']:7} {s['undecided']:13}")
        for why, n in sorted(s["undecided_why"].items(), key=lambda x: -x[1]):
            print(f"{'':58}not decidable: {why} {n}")
        for why, n in sorted(s["exempt"].items()):
            print(f"{'':58}confirmed, {why} in the class file: {n}")
    total = sum(s["findings"] for s in rules.values())
    decided = sum(s["confirmed"] + s["refuted"] for s in rules.values())
    with_oracle = sum(s["findings"] for s in rules.values() if s["oracle"])
    print()
    for e in sorted(refuted, key=lambda e: (e["rule"], e["artifact"], e["file"], e["line"])):
        print(f"REFUTED  {e['artifact']}  {e['file']}:{e['line']}  {e['rule']}: {e['detail']}")
    print()
    share = f"{100 * decided / total:.1f}%" if total else "-"
    print(f"findings {total}, of rules with an oracle {with_oracle}, decided {decided} ({share} of all)")
    print(f"artifacts: {len(arts)} run, {len(skipped)} skipped, {len(failed)} failed, "
          f"{time.monotonic() - start:.0f} s")
    report = {"rules": rules, "artifacts": arts, "refuted": refuted, "skipped": skipped, "failed": failed}
    with open(os.path.join(out_dir, "report.json"), "w") as fh:
        json.dump(report, fh, indent=1, sort_keys=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
