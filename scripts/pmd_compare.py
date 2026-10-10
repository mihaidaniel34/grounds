#!/usr/bin/env python3
"""Compare grounds with PMD on the rules grounds maps to a PMD rule.

    scripts/pmd_compare.py --pmd path/to/pmd-bin-7.x/bin/pmd PROJECT_DIR...

Runs PMD (source only, no build) with exactly the mapped rules and grounds with
the corresponding rule ids, matches findings by file and line (within --slack
lines), and prints per PMD rule how many both report, only grounds reports and
only PMD reports. Samples of every disagreement land in target/pmd-compare/.
"""
import argparse, collections, csv, hashlib, io, json, os, re, subprocess, sys, tempfile, zipfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def pmd_categories(pmd):
    """Rule name -> category, read from the pmd-java jar of the install."""
    lib = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(pmd))), "lib")
    jar = next(os.path.join(lib, f) for f in os.listdir(lib) if re.match(r"pmd-java-\d.*\.jar$", f))
    cats = {}
    with zipfile.ZipFile(jar) as z:
        for n in z.namelist():
            m = re.match(r"category/java/(\w+)\.xml$", n)
            if m:
                for rule in re.findall(r'<rule\s+name="(\w+)"', z.read(n).decode()):
                    cats[rule] = m.group(1)
    return cats


def is_test(path):
    return "/src/test/" in "/" + path or re.search(r"(^|/)(test|tests|testFixtures|integrationTest)/", path) is not None


def mapping(grounds):
    """PMD rule -> grounds rule ids that cite it."""
    rules = json.loads(subprocess.run([grounds, "rules", "--json"], capture_output=True, text=True, check=True).stdout)
    out = collections.defaultdict(list)
    for r in rules:
        for s in r["sources"]:
            if s.startswith("pmd:"):
                out[s[4:]].append(r["id"])
    return out


def run_pmd(pmd, project, ruleset, threads):
    p = subprocess.run([pmd, "check", "-d", project, "-R", ruleset, "-f", "csv", "--no-progress", "--no-cache",
                        "-t", str(threads), "--relativize-paths-with", project],
                       capture_output=True, text=True, env={**os.environ, "PMD_JAVA_OPTS": os.environ.get("PMD_JAVA_OPTS", "-Xmx6g")})
    # 1 and 5 also cover files PMD could not process; the rest is still analyzed
    if p.returncode not in (0, 1, 4, 5) or not p.stdout.startswith('"Problem"'):
        sys.exit(f"pmd failed on {project}:\n{p.stderr[-2000:]}")
    rows = list(csv.DictReader(io.StringIO(p.stdout)))
    errors = len(re.findall(r"^\[ERROR\] at ", p.stderr, re.M))
    return [(r["File"].replace(os.sep, "/").lstrip("./"), int(r["Line"]), r["Rule"], r["Description"]) for r in rows], errors


def run_grounds(grounds, project, ids):
    p = subprocess.run([grounds, "check", "--root", project, "--format", "json", "--rules", ",".join(ids)],
                       capture_output=True, text=True, stdin=subprocess.DEVNULL)
    return [(f["path"], f["line"], f["rule"], f["message"]) for f in json.loads(p.stdout or "[]")]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pmd", default=os.environ.get("PMD", "pmd"))
    ap.add_argument("--grounds", default=os.path.join(REPO, "target/release/grounds"))
    ap.add_argument("--slack", type=int, default=3, help="line distance that still counts as the same finding")
    ap.add_argument("--rules", help="only these PMD rules (comma separated)")
    ap.add_argument("--threads", type=int, default=os.cpu_count())
    ap.add_argument("--out", default=os.path.join(REPO, "target/pmd-compare"))
    ap.add_argument("projects", nargs="+")
    a = ap.parse_args()

    cats = pmd_categories(a.pmd)
    by_pmd = mapping(a.grounds)
    if a.rules:
        by_pmd = {k: v for k, v in by_pmd.items() if k in a.rules.split(",")}
    unknown = sorted(r for r in by_pmd if r not in cats)
    if unknown:
        print("not PMD rules (fix the source metadata):", ", ".join(unknown), file=sys.stderr)
    by_pmd = {k: v for k, v in by_pmd.items() if k in cats}
    # Rows are connected groups of PMD and grounds rules (several PMD rules
    # can map to one grounds rule and vice versa); within a group any grounds
    # finding can match any PMD finding.
    parent = {}

    def find(x):
        while parent.setdefault(x, x) != x:
            x = parent[x]
        return x

    for p, ids in by_pmd.items():
        for i in ids:
            parent[find(("p", p))] = find(("g", i))
    groups = collections.defaultdict(lambda: ([], []))
    for p, ids in by_pmd.items():
        groups[find(("p", p))][0].append(p)
        for i in ids:
            if i not in groups[find(("p", p))][1]:
                groups[find(("p", p))][1].append(i)
    label = {k: "+".join(sorted(v[0])) for k, v in groups.items()}
    group_of_pmd = {p: label[find(("p", p))] for p in by_pmd}
    group_of_grounds = {i: label[find(("g", i))] for ids in by_pmd.values() for i in ids}
    grounds_of = {label[k]: sorted(v[1]) for k, v in groups.items()}

    with tempfile.NamedTemporaryFile("w", suffix=".xml", delete=False) as f:
        f.write('<?xml version="1.0"?>\n<ruleset name="grounds-mapped" xmlns="http://pmd.sourceforge.net/ruleset/2.0.0">\n'
                '<description>PMD rules grounds maps to</description>\n')
        for r in sorted(by_pmd):
            f.write(f'<rule ref="category/java/{cats[r]}.xml/{r}"/>\n')
        f.write("</ruleset>\n")
        ruleset = f.name
    ruleset_hash = hashlib.sha1(open(ruleset, "rb").read()).hexdigest()[:12]

    stats = collections.defaultdict(collections.Counter)
    samples = collections.defaultdict(lambda: collections.defaultdict(list))

    def part(path, key):
        return "test_" + key if is_test(path) else key

    os.makedirs(os.path.join(a.out, "pmd-cache"), exist_ok=True)
    for project in a.projects:
        project = os.path.abspath(project)
        name = os.path.basename(project.rstrip("/"))
        head = subprocess.run(["git", "-C", project, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        cache = os.path.join(a.out, "pmd-cache", f"{name}-{head[:12]}-{ruleset_hash}.json")
        if os.path.exists(cache):
            pf, perr = json.load(open(cache))
        else:
            pf, perr = run_pmd(a.pmd, project, ruleset, a.threads)
            json.dump([pf, perr], open(cache, "w"))
        gf = run_grounds(a.grounds, project, sorted(group_of_grounds))
        print(f"{name}: pmd {len(pf)} ({perr} files PMD could not process), grounds {len(gf)}", file=sys.stderr)
        # (file, group) -> [line, message, matched]
        pidx = collections.defaultdict(list)
        for path, line, rule, msg in pf:
            pidx[(path, group_of_pmd[rule])].append([line, f"[{rule}] {msg}", False])
        for path, line, gid, msg in gf:
            g = group_of_grounds[gid]
            near = [e for e in pidx[(path, g)] if abs(e[0] - line) <= a.slack]
            if near:
                hit = min(near, key=lambda e: (e[2], abs(e[0] - line)))
                if not hit[2]:
                    stats[g][part(path, "both")] += 1
                hit[2] = True
            else:
                stats[g][part(path, "grounds_only")] += 1
                samples[g][part(path, "grounds_only")].append(f"{name}/{path}:{line} [{gid}] {msg}")
        for (path, g), es in pidx.items():
            for line, msg, matched in es:
                if not matched:
                    stats[g][part(path, "pmd_only")] += 1
                    samples[g][part(path, "pmd_only")].append(f"{name}/{path}:{line} {msg}")
    os.unlink(ruleset)

    def total(g):
        return sum(stats[g].values())

    rows = sorted(grounds_of, key=lambda g: -total(g))
    w = min(max(map(len, rows)), 48) if rows else 10
    print(f"{'':<{w}}  {'main code':^23}  {'test code':^23}")
    print(f"{'PMD rule':<{w}}  {'both':>7}{'grounds':>8}{'pmd':>8}  {'both':>7}{'grounds':>8}{'pmd':>8}  grounds rules")
    for g in rows:
        if not total(g):
            continue
        s = stats[g]
        print(f"{g[:w]:<{w}}  {s['both']:>7}{s['grounds_only']:>8}{s['pmd_only']:>8}  "
              f"{s['test_both']:>7}{s['test_grounds_only']:>8}{s['test_pmd_only']:>8}  {','.join(grounds_of[g])}")
    silent = [g for g in rows if not total(g)]
    if silent:
        print(f"\nno findings from either tool: {', '.join(silent)}")
    with open(os.path.join(a.out, "summary.json"), "w") as f:
        json.dump({g: {"grounds": grounds_of[g], **stats[g]} for g in rows}, f, indent=1)
    with open(os.path.join(a.out, "samples.json"), "w") as f:
        json.dump(samples, f, indent=1)


if __name__ == "__main__":
    main()
