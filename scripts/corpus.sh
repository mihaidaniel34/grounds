#!/usr/bin/env bash
# Regression corpus: real projects at pinned commits.
#
#   scripts/corpus.sh fetch      clone missing projects, check out the pinned commits
#   scripts/corpus.sh classpath  `grounds classpath` in each project (Maven/Gradle
#                                resolve the exact classpath) and scan_caches = false
#   scripts/corpus.sh check      hard checks + findings snapshot (diffed against the baseline)
#   scripts/corpus.sh baseline   accept the last snapshot as the new baseline
#
# Hard checks fail the run: zero parse errors, zero findings from rules that
# claim the code would not compile or the Spring context would not start, the
# same findings (every field) on one thread and on many, and every planted
# bug still reported. Everything else is a snapshot diff to read.
#
# CORPUS_DIR (default ~/grounds-testbed) holds the checkouts; GROUNDS points at
# the binary (default target/release/grounds, built if missing).
# CORPUS_ONLY=a,b limits fetch/check to those projects (one CI job each).
# CORPUS_BASELINE=dir compares with dir/<project>.txt instead of the local
# target/corpus/baseline (CI keeps its own baseline in corpus/ci: runners
# have no Maven/Gradle caches, so fewer calls resolve than on a dev machine).
# CORPUS_SHALLOW=1 fetches just the pinned commit (CI), or with CORPUS_DEPTH=n
# the n commits that end there (for scripts/replay.py).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
corpus_dir=${CORPUS_DIR:-$HOME/grounds-testbed}
grounds=${GROUNDS:-$repo/target/release/grounds}
case $grounds in /*) ;; *) grounds=$PWD/$grounds ;; esac  # classpath runs it from each project
out=$repo/target/corpus
projects=$repo/corpus/projects.tsv

hard_rules=unresolved-method,missing-jdk-import,unresolved-constructor,unresolved-field,override-matches-nothing,unimplemented-abstract-method,bean-not-found,ambiguous-bean,qualifier-not-found,bean-name-conflict,circular-constructor-dependency
planted_rules=bean-not-found,ambiguous-bean,qualifier-not-found,bean-name-conflict,circular-constructor-dependency,lombok-constructor-drops-qualifier

# path:line rule, one per finding, sorted
findings() {
  { "$grounds" check --root "$1" --format json ${2:+--rules "$2"} 2>/dev/null < /dev/null || true; } |
    python3 -c 'import json,sys
for f in json.load(sys.stdin): print("%s:%s %s" % (f["path"], f["line"], f["rule"]))' | LC_ALL=C sort
}

# every field of every finding, one JSON object per line, sorted; on $2
# threads, or on the default number when $2 is empty
all_fields() {
  { env ${2:+RAYON_NUM_THREADS=$2} "$grounds" check --root "$1" --format json 2>/dev/null < /dev/null || true; } |
    python3 -c 'import json,sys
for f in json.load(sys.stdin): print(json.dumps(f, sort_keys=True))' | LC_ALL=C sort
}

# all_fields output as the snapshot's lines: path:line rule, sorted
snapshot_lines() {
  python3 -c 'import json,sys
for l in sys.stdin:
    f = json.loads(l); print("%s:%s %s" % (f["path"], f["line"], f["rule"]))' | LC_ALL=C sort
}

selected() { [ -z "${CORPUS_ONLY:-}" ] || [[ ",$CORPUS_ONLY," == *",$1,"* ]]; }

fetch() {
  mkdir -p "$corpus_dir"
  while IFS=$'\t' read -r name url commit <&3; do
    selected "$name" || continue
    dir=$corpus_dir/$name
    if [ ! -d "$dir/.git" ] && [ -n "${CORPUS_SHALLOW:-}" ]; then
      echo "fetch $name"
      git init -q "$dir" && git -C "$dir" fetch -q --depth "${CORPUS_DEPTH:-1}" "$url" "$commit" && git -C "$dir" checkout -q --detach FETCH_HEAD
      continue
    fi
    if [ ! -d "$dir/.git" ]; then
      echo "clone $name"
      git clone --quiet --filter=blob:none "$url" "$dir"
    fi
    if [ "$(git -C "$dir" rev-parse HEAD)" != "$commit" ]; then
      git -C "$dir" fetch --quiet origin "$commit" 2>/dev/null || git -C "$dir" fetch --quiet origin
      git -C "$dir" checkout --quiet --detach "$commit"
      echo "$name -> ${commit:0:12}"
    fi
  done 3< "$projects"
}

# the exact classpath through the build tool, and nothing else
classpath() {
  [ -x "$grounds" ] || cargo build --release --quiet --manifest-path "$repo/Cargo.toml" -p grounds-cli
  failed=0
  while IFS=$'\t' read -r name _ _ <&3; do
    selected "$name" || continue
    dir=$corpus_dir/$name
    # CI checkouts are never built: let Maven resolve sibling modules
    # through the reactor (an opt-in, it runs early-phase plugins)
    flags=()
    [ -f "$dir/pom.xml" ] && [ ! -f "$dir/gradlew" ] && "$grounds" classpath --help | command grep -q -- --maven-lifecycle && flags=(--maven-lifecycle)
    if ! (cd "$dir" && "$grounds" classpath --root . --color never ${flags[@]+"${flags[@]}"} < /dev/null); then
      echo "$name: grounds classpath FAILED"; failed=1; continue
    fi
    [ -e "$dir/grounds.toml" ] && { echo "$name: has its own grounds.toml"; failed=1; continue; }
    printf '[classpath]\nscan_caches = false\n' > "$dir/grounds.toml"
    echo "$name: $(wc -l < "$dir/.grounds/classpath.txt" | tr -d ' ') jars"
  done 3< "$projects"
  exit $failed
}

check() {
  [ -x "$grounds" ] || cargo build --release --quiet --manifest-path "$repo/Cargo.toml" -p grounds-cli
  errors=${ERRORS:-$repo/target/release/examples/errors}
  [ -n "${ERRORS:-}" ] || cargo build --release --quiet --manifest-path "$repo/Cargo.toml" -p grounds-syntax --example errors
  rm -rf "$out/current" "$out"/threads-*.diff && mkdir -p "$out/current"
  failed=0

  while IFS=$'\t' read -r name _ commit <&3; do
    selected "$name" || continue
    dir=$corpus_dir/$name
    if [ ! -d "$dir" ]; then echo "$name: missing (run fetch)"; failed=1; continue; fi
    head=$(git -C "$dir" rev-parse HEAD)
    [ "$head" = "$commit" ] || echo "$name: at ${head:0:12}, pinned ${commit:0:12}"

    # Classes are resolved and files analyzed on all cores: what is reported
    # must not depend on how many there are or on which thread got there
    # first. Every field of every finding is compared between the default
    # number of threads (the snapshot), one, and more than a runner has cores.
    all_fields "$dir" "" > "$out/threads-default.txt"
    all_fields "$dir" 1 > "$out/threads-1.txt"
    all_fields "$dir" 16 > "$out/threads-16.txt"
    snapshot_lines < "$out/threads-default.txt" > "$out/current/$name.txt"
    { diff -u --label "1 thread" --label "16 threads" "$out/threads-1.txt" "$out/threads-16.txt"
      diff -u --label "default threads" --label "1 thread" "$out/threads-default.txt" "$out/threads-1.txt"; } > "$out/threads-$name.diff" || true
    # (the first twenty: `head` would close the pipe on `sort`, which fails the script under pipefail)
    threads=$(sed -n 's/^[-+]\({.*\)/\1/p' "$out/threads-$name.diff" | LC_ALL=C sort -u | sed -n '1,20p')
    [ -n "$threads" ] || rm -f "$out/threads-$name.diff"
    rm -f "$out/threads-default.txt" "$out/threads-1.txt" "$out/threads-16.txt"
    hard=$(awk -v rules=",$hard_rules," 'index(rules, "," $2 ",")' "$out/current/$name.txt")
    parse=$(cd "$dir" && find . -name '*.java' -not -path '*/.grounds/*' -print0 | xargs -0 "$errors")
    calls=$("$grounds" type-stats "$dir" --color never 2>/dev/null < /dev/null | sed -n 's/^calls resolved //p')
    echo "$name	$calls" >> "$out/current/stats.tsv"
    printf '%-28s %6d findings, calls resolved %s' "$name" "$(wc -l < "$out/current/$name.txt")" "$calls"
    if [ -n "$hard" ] || [ -n "$parse" ] || [ -n "$threads" ]; then
      echo "  FAIL"; failed=1
      [ -n "$hard" ] && sed 's/^/    /' <<< "$hard" | head -20
      [ -n "$parse" ] && sed 's/^/    parse: /' <<< "$parse" | head -20
      [ -n "$threads" ] && sed 's/^/    differs with the number of threads: /' <<< "$threads"
    else
      echo
    fi
  done 3< "$projects"

  # Planted bugs: apply the patch to a copy and expect exactly the listed findings.
  for patch in "$repo"/corpus/planted/*.patch; do
    name=$(basename "$patch" .patch)
    selected "$name" || continue
    tmp=$(mktemp -d)
    git -C "$corpus_dir/$name" archive HEAD | tar -x -C "$tmp"
    git -C "$tmp" apply "$patch" 2>/dev/null || (cd "$tmp" && patch -p1 --quiet < "$patch")
    if diff <(findings "$tmp" "$planted_rules") "$repo/corpus/planted/$name.expected" > "$out/planted-$name.diff"; then
      echo "planted $name: $(wc -l < "$repo/corpus/planted/$name.expected") expected findings reported"
    else
      echo "planted $name: FAIL (< reported, > expected)"; sed 's/^/    /' "$out/planted-$name.diff"; failed=1
    fi
    rm -rf "$tmp"
  done

  if [ -n "${CORPUS_BASELINE:-}" ]; then
    : > "$out/snapshot.diff"
    for f in "$out"/current/*.txt; do
      b=$CORPUS_BASELINE/$(basename "$f")
      [ -f "$b" ] || { echo "snapshot: no baseline for $(basename "$f" .txt) in $CORPUS_BASELINE"; b=/dev/null; }
      diff -u "$b" "$f" --label "baseline/$(basename "$f")" --label "current/$(basename "$f")" >> "$out/snapshot.diff" || true
    done
    if [ -s "$out/snapshot.diff" ]; then
      echo "snapshot: $(command grep -c '^-[^-]' "$out/snapshot.diff") removed, $(command grep -c '^+[^+]' "$out/snapshot.diff") added (target/corpus/snapshot.diff)"
    else
      echo "snapshot: unchanged"
    fi
  elif [ -d "$out/baseline" ]; then
    if diff -r -u "$out/baseline" "$out/current" > "$out/snapshot.diff"; then
      echo "snapshot: unchanged"
    else
      echo "snapshot: $(command grep -c '^-[^-]' "$out/snapshot.diff") removed, $(command grep -c '^+[^+]' "$out/snapshot.diff") added (target/corpus/snapshot.diff)"
    fi
  else
    echo "snapshot: no baseline yet (scripts/corpus.sh baseline)"
  fi
  exit $failed
}

case ${1:-check} in
  fetch) fetch ;;
  classpath) classpath ;;
  check) check ;;
  baseline) rm -rf "$out/baseline" && cp -R "$out/current" "$out/baseline" && echo "baseline updated" ;;
  *) echo "usage: $0 fetch|check|baseline" >&2; exit 2 ;;
esac
