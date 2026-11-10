#!/usr/bin/env python3
"""Smoke corpus: does grounds survive code it was never tuned on?

The regression corpus (corpus/projects.tsv) is what the rules were tuned on.
The smoke corpus (corpus/smoke.tsv) is everything else: about 150 projects at
pinned commits, picked for variety (build tools, Java 8 to 25, frameworks,
generated code, very large files, Android, compilers and parsers, old code).
Nothing about the findings is judged except crashes:

  fail      a panic on stderr (also one a rule's guard caught), a run over the
            time limit, an exit status other than 0 or 1 (1 means "error-level
            findings") or output that is not the JSON findings list, and a
            project that could not be fetched (the list needs fixing)
  report    parse errors (the `errors` example of grounds-syntax: where
            tree-sitter could not read a file), findings of the compile-break
            rules (`hard_rules` in scripts/corpus.sh; on code that builds these
            are suspects, not failures), wall time, peak memory, files

For each project: a shallow fetch of the pinned commit only, a grounds.toml
with `[classpath] scan_caches = false` if it has none (so jars in the local
Maven/Gradle caches do not change the result), then
`grounds check --root DIR --format json`. Checkouts are deleted afterwards
unless --keep is given; with --keep an existing checkout at the pinned commit
is reused.

  python3 -I scripts/smoke.py [--only a,b] [--shard 1/10] [--timeout 600]
      [--dir ~/grounds-smoke] [--grounds target/release/grounds]
      [--errors target/release/examples/errors] [--out report.json] [--keep]
  python3 -I scripts/smoke.py --summary report.json...   (markdown tables)

Exit status 1 on any failure as defined above.
"""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PANIC_RE = re.compile(r"^thread '(.*?)'(?: \(\d+\))? panicked at (\S+?):?$")
RULE_PANIC_RE = re.compile(r"^grounds: rule (\S+) panicked on (.+)$")
OVERFLOW_RE = re.compile(r"^thread '(.*)' \(\d+\) has overflowed its stack$|^thread '(.*)' has overflowed its stack$")
ERROR_LINE_RE = re.compile(r"^(.*?):(\d+):(\d+): (error|missing [^:]*): (.*)$")
KEEP_ERRORS = 200  # files with parse errors listed per project in the report (first error of each)


def hard_rules():
    """`hard_rules` from scripts/corpus.sh, the one list of compile-break and DI rules."""
    with open(os.path.join(REPO, "scripts", "corpus.sh")) as fh:
        for line in fh:
            m = re.match(r"^hard_rules=(\S+)", line)
            if m:
                return set(m.group(1).split(","))
    sys.exit("scripts/corpus.sh: no hard_rules line")


def projects(path):
    out = []
    with open(path) as fh:
        for line in fh:
            line = line.rstrip("\n")
            if not line.strip() or line.startswith("#"):
                continue
            name, url, commit = line.split("\t")[:3]
            out.append({"name": name, "url": url, "commit": commit})
    return out


def git(d, *args, timeout=None):
    return subprocess.run(["git", "-C", d, *args], capture_output=True, text=True, timeout=timeout)


def fetch(p, d, keep):
    """Check out p's pinned commit in d; None or an error message."""
    if keep and os.path.isdir(os.path.join(d, ".git")) and git(d, "rev-parse", "HEAD").stdout.strip() == p["commit"]:
        return None
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    err = ""
    for attempt in range(3):
        if attempt:
            time.sleep(5 * attempt)
        if git(d, "init", "-q").returncode:
            return "git init failed"
        try:
            r = git(d, "fetch", "-q", "--depth", "1", "--no-tags", p["url"], p["commit"], timeout=1800)
            if r.returncode == 0:
                r = git(d, "-c", "advice.detachedHead=false", "checkout", "-q", "--detach", "FETCH_HEAD", timeout=1800)
                if r.returncode == 0:
                    return None
            err = (r.stderr or "").strip()[-500:]
        except subprocess.TimeoutExpired as e:
            err = f"git timed out after {e.timeout} s"
            shutil.rmtree(d, ignore_errors=True)
            os.makedirs(d)
    return f"fetch failed: {err}"


def java_files(d):
    out = []
    for dirpath, dirnames, filenames in os.walk(d):
        dirnames[:] = [n for n in dirnames if n not in (".git", ".grounds")]
        out.extend(os.path.join(dirpath, f) for f in filenames if f.endswith(".java"))
    return sorted(out)


def tree_bytes(d):
    total = 0
    for dirpath, dirnames, filenames in os.walk(d):
        if dirpath == d:
            dirnames[:] = [n for n in dirnames if n != ".grounds"]
        for f in filenames:
            try:
                total += os.lstat(os.path.join(dirpath, f)).st_size
            except OSError:
                pass
    return total


def run_limited(cmd, timeout, stdout, stderr, cwd=None):
    """(exit status or None on timeout, signal or None, wall seconds, peak RSS
    in MiB or None). Peak memory comes from wait4's rusage: Linux reports
    kilobytes, macOS bytes."""
    t0 = time.monotonic()
    proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, cwd=cwd, start_new_session=True)
    deadline = t0 + timeout
    timed_out = False
    while True:
        pid, status, ru = os.wait4(proc.pid, os.WNOHANG)
        if pid:
            break
        if time.monotonic() > deadline:
            timed_out = True
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            pid, status, ru = os.wait4(proc.pid, 0)
            break
        time.sleep(0.05)
    proc.returncode = 0  # reaped here, keep Popen from waiting again
    wall = time.monotonic() - t0
    rss = ru.ru_maxrss / (1024 * 1024 if sys.platform == "darwin" else 1024)
    if timed_out:
        return None, None, wall, round(rss)
    if os.WIFSIGNALED(status):
        return None, os.WTERMSIG(status), wall, round(rss)
    return os.WEXITSTATUS(status), None, wall, round(rss)


def panics(stderr):
    """[{location, message, thread}] for each Rust panic on stderr, plus the
    rule and file when a rule's guard caught it."""
    lines = stderr.splitlines()
    out = []
    for i, line in enumerate(lines):
        m = PANIC_RE.match(line)
        if m:
            msg = []
            for nxt in lines[i + 1:i + 6]:
                if nxt.startswith("note:") or PANIC_RE.match(nxt) or RULE_PANIC_RE.match(nxt) or not nxt.strip():
                    break
                msg.append(nxt)
            out.append({"thread": m.group(1), "location": m.group(2), "message": "\n".join(msg)[:500]})
            continue
        m = OVERFLOW_RE.match(line)
        if m:
            out.append({"thread": m.group(1) or m.group(2), "location": "stack overflow", "message": line})
            continue
        m = RULE_PANIC_RE.match(line)
        if m:
            # attach to the latest panic without a rule (the hook prints first)
            for p in reversed(out):
                if "rule" not in p:
                    p["rule"], p["file"] = m.group(1), m.group(2)
                    break
            else:
                out.append({"rule": m.group(1), "file": m.group(2), "location": "?", "message": ""})
    return out


def parse_errors(errors_bin, d, files, timeout):
    """Lines of the `errors` example for all Java files, paths relative to d."""
    lines, rels = [], [os.path.relpath(f, d) for f in files]
    t_end = time.monotonic() + timeout
    for i in range(0, len(rels), 500):
        left = t_end - time.monotonic()
        if left <= 0:
            return lines, "parse error scan over the time limit"
        try:
            r = subprocess.run([errors_bin, *rels[i:i + 500]], cwd=d, capture_output=True, text=True, errors="replace",
                               timeout=left)
        except subprocess.TimeoutExpired:
            return lines, "parse error scan over the time limit"
        if r.returncode:
            return lines, f"errors example exited {r.returncode}: {r.stderr.strip()[-300:]}"
        lines.extend(r.stdout.splitlines())
    return lines, None


def check(p, d, a, hard):
    res = {"name": p["name"], "url": p["url"], "commit": p["commit"], "failures": []}
    own = os.path.exists(os.path.join(d, "grounds.toml"))
    res["own_config"] = own
    if not own:
        with open(os.path.join(d, "grounds.toml"), "w") as fh:
            fh.write("[classpath]\nscan_caches = false\n")
    files = java_files(d)
    res["java_files"] = len(files)
    res["java_bytes"] = sum(os.path.getsize(f) for f in files)
    res["checkout_mb"] = round(tree_bytes(d) / 1e6)
    out_path, err_path = d + ".stdout.json", d + ".stderr.txt"
    with open(out_path, "wb") as so, open(err_path, "wb") as se:
        status, sig, wall, rss = run_limited([a.grounds, "check", "--root", d, "--format", "json"], a.timeout, so, se)
    with open(err_path, encoding="utf-8", errors="replace") as fh:
        stderr = fh.read()
    res.update(exit=status, signal=sig, wall_s=round(wall, 2), peak_rss_mb=rss)
    res["files_per_s"] = round(len(files) / wall) if wall > 0 else None
    res["panics"] = panics(stderr)
    findings = None
    try:
        with open(out_path, encoding="utf-8") as fh:
            findings = json.load(fh)
        if not isinstance(findings, list):
            findings = None
    except (OSError, ValueError):
        pass
    for f in (out_path, err_path):
        try:
            os.remove(f)
        except OSError:
            pass
    if status is None and sig is None:
        res["failures"].append(f"timeout after {a.timeout} s")
    elif sig is not None:
        res["failures"].append(f"killed by signal {sig}")
    elif status not in (0, 1):
        res["failures"].append(f"exit status {status}")
    elif findings is None:
        res["failures"].append("stdout is not a JSON findings list")
    elif status == 1 and not any(f.get("severity") == "error" for f in findings):
        res["failures"].append("exit status 1 without error-level findings")
    if res["panics"]:
        res["failures"].append(f"{len(res['panics'])} panic(s)")
    if res["failures"] and (status not in (0, 1) or findings is None):
        res["stderr_tail"] = stderr[-3000:]
    if findings is not None:
        res["findings"] = len(findings)
        res["by_severity"] = {s: sum(1 for f in findings if f.get("severity") == s) for s in ("error", "warning", "info")}
        res["compile_break_suspects"] = [f"{f['path']}:{f['line']} {f['rule']}: {f.get('message', '')}"
                                         for f in findings if f.get("rule") in hard]
    errs, err = parse_errors(a.errors, d, files, a.timeout)
    if err:
        res["failures"].append(err)
    res["parse_errors"] = len(errs)
    first = {}
    for line in errs:
        m = ERROR_LINE_RE.match(line)
        if m:
            first.setdefault(m.group(1), line)
    res["parse_error_files"] = len(first)
    res["parse_error_lines"] = sorted(first.values())[:KEEP_ERRORS]
    return res


def line(r):
    if r.get("fetch_error"):
        return f"{r['name']:32} FETCH FAILED  {r['fetch_error'][:200]}"
    s = (f"{r['name']:32} {r['java_files']:6} files {r['wall_s']:7.1f} s {r['peak_rss_mb'] or 0:6} MiB "
         f"{r.get('findings', '-'):>7} findings, {len(r.get('compile_break_suspects', []))} compile-break, "
         f"{r['parse_errors']} parse errors")
    if r["failures"]:
        s += "  FAIL: " + "; ".join(r["failures"])
        for p in r["panics"][:5]:
            s += f"\n    panic at {p['location']}{' in rule ' + p['rule'] + ' on ' + p['file'] if 'rule' in p else ''}: {p['message'][:200]}"
        if r.get("stderr_tail"):
            s += "\n    " + "\n    ".join(r["stderr_tail"].strip().splitlines()[-8:])
    return s


def summary(paths):
    rows = []
    for path in paths:
        with open(path) as fh:
            rows.extend(json.load(fh)["projects"])
    rows.sort(key=lambda r: r["name"])
    ran = [r for r in rows if not r.get("fetch_error")]
    failed = [r for r in rows if r.get("failures") or r.get("fetch_error")]
    print(f"**{len(ran)} projects run**, {sum(r['java_files'] for r in ran):,} Java files; "
          f"panics in {sum(1 for r in ran if r['panics'])}, "
          f"timeouts {sum(1 for r in ran if any(f.startswith('timeout') for f in r['failures']))}, "
          f"failed {len(failed)}, fetch failures {len(rows) - len(ran)}, "
          f"parse errors {sum(r['parse_errors'] for r in ran)} in {sum(r['parse_error_files'] for r in ran)} files, "
          f"compile-break suspects {sum(len(r.get('compile_break_suspects', [])) for r in ran)}\n")
    if failed:
        print("| failed | why |\n|---|---|")
        for r in failed:
            why = r.get("fetch_error") or "; ".join(r["failures"])
            for p in r.get("panics", [])[:3]:
                why += f"<br>`{p['location']}`{' (' + p['rule'] + ')' if 'rule' in p else ''}: {p['message'][:120]}"
            print(f"| {r['name']} | {why.replace('|', '/')} |")
        print()
    print("| project | Java files | time (s) | files/s | peak MiB | findings | compile-break | parse errors |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    for r in ran:
        print(f"| {r['name']} | {r['java_files']} | {r['wall_s']} | {r['files_per_s']} | {r['peak_rss_mb']} | "
              f"{r.get('findings', '-')} | {len(r.get('compile_break_suspects', []))} | {r['parse_errors']} |")
    return 0


def main():
    if len(sys.argv) >= 3 and sys.argv[1] == "--summary":
        return summary(sys.argv[2:])
    ap = argparse.ArgumentParser()
    ap.add_argument("--list", default=os.path.join(REPO, "corpus", "smoke.tsv"))
    ap.add_argument("--only", help="comma-separated project names")
    ap.add_argument("--shard", help="i/n: every n-th project starting at the i-th (1-based)")
    ap.add_argument("--timeout", type=int, default=600, help="seconds per grounds run (default 600)")
    ap.add_argument("--dir", default=os.environ.get("SMOKE_DIR", os.path.expanduser("~/grounds-smoke")))
    ap.add_argument("--grounds", default=os.path.join(REPO, "target", "release", "grounds"))
    ap.add_argument("--errors", default=os.path.join(REPO, "target", "release", "examples", "errors"))
    ap.add_argument("--out", help="write the JSON report here")
    ap.add_argument("--keep", action="store_true", help="keep (and reuse) checkouts")
    a = ap.parse_args()
    a.grounds, a.errors, a.dir = os.path.abspath(a.grounds), os.path.abspath(a.errors), os.path.abspath(a.dir)
    for b in (a.grounds, a.errors):
        if not os.access(b, os.X_OK):
            sys.exit(f"{b}: not found (cargo build --release -p grounds-cli; "
                     "cargo build --release -p grounds-syntax --example errors)")
    todo = projects(a.list)
    if a.only:
        names = set(a.only.split(","))
        unknown = names - {p["name"] for p in todo}
        if unknown:
            sys.exit(f"not in {a.list}: {', '.join(sorted(unknown))}")
        todo = [p for p in todo if p["name"] in names]
    if a.shard:
        i, n = (int(x) for x in a.shard.split("/"))
        if not 1 <= i <= n:
            sys.exit("--shard i/n with 1 <= i <= n")
        todo = todo[i - 1::n]
    hard = hard_rules()
    os.makedirs(a.dir, exist_ok=True)
    version = subprocess.run([a.grounds, "--version"], capture_output=True, text=True).stdout.strip()
    print(f"{version}: {len(todo)} projects, time limit {a.timeout} s", flush=True)

    # fetch the next project while the current one is checked
    fetched = {}

    def fetch_into(k):
        p = todo[k]
        t0 = time.monotonic()
        try:
            err = fetch(p, os.path.join(a.dir, p["name"]), a.keep)
        except Exception as e:  # noqa: BLE001 - any error is this project's fetch failure, not the run's
            err = f"fetch failed: {e!r}"
        fetched[k] = (err, time.monotonic() - t0)

    results = []
    nxt = threading.Thread(target=fetch_into, args=(0,)) if todo else None
    if nxt:
        nxt.start()
    for k, p in enumerate(todo):
        nxt.join()
        if k + 1 < len(todo):
            nxt = threading.Thread(target=fetch_into, args=(k + 1,))
            nxt.start()
        d = os.path.join(a.dir, p["name"])
        err, fetch_s = fetched.pop(k)
        if err:
            r = {"name": p["name"], "url": p["url"], "commit": p["commit"], "fetch_error": err, "failures": [err]}
        else:
            r = check(p, d, a, hard)
            r["fetch_s"] = round(fetch_s, 1)
        results.append(r)
        print(line(r), flush=True)
        if not a.keep:
            shutil.rmtree(d, ignore_errors=True)
        if a.out:  # rewritten after every project: a cancelled run keeps what it measured
            with open(a.out, "w") as fh:
                json.dump({"grounds": version, "timeout_s": a.timeout, "projects": results}, fh, indent=1)
    failed = [r for r in results if r["failures"]]
    print(f"{len(results)} projects, {len(failed)} failed" + (": " + ", ".join(r["name"] for r in failed) if failed else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
