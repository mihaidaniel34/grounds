#!/usr/bin/env bash
# Regression corpus: real projects at pinned commits.
#
#   scripts/corpus.sh fetch      clone missing projects, check out the pinned commits
#   scripts/corpus.sh check      hard checks + findings snapshot (diffed against the baseline)
#   scripts/corpus.sh baseline   accept the last snapshot as the new baseline
#
# Hard checks fail the run: zero parse errors, zero findings from rules that
# claim the code would not compile or the Spring context would not start, and
# every planted bug still reported. Everything else is a snapshot diff to read.
#
# CORPUS_DIR (default ~/grounds-testbed) holds the checkouts; GROUNDS points at
# the binary (default target/release/grounds, built if missing).
set -euo pipefail

repo=$(cd "$(dirname "$0")/.." && pwd)
corpus_dir=${CORPUS_DIR:-$HOME/grounds-testbed}
grounds=${GROUNDS:-$repo/target/release/grounds}
out=$repo/target/corpus
projects=$repo/corpus/projects.tsv

hard_rules=unresolved-method,unresolved-constructor,unresolved-field,override-matches-nothing,unimplemented-abstract-method,bean-not-found,ambiguous-bean,qualifier-not-found,bean-name-conflict,circular-constructor-dependency
planted_rules=bean-not-found,ambiguous-bean,qualifier-not-found,bean-name-conflict,circular-constructor-dependency,lombok-constructor-drops-qualifier

# path:line rule, one per finding, sorted
findings() {
  { "$grounds" check --root "$1" --format json ${2:+--rules "$2"} 2>/dev/null < /dev/null || true; } |
    python3 -c 'import json,sys
for f in json.load(sys.stdin): print("%s:%s %s" % (f["path"], f["line"], f["rule"]))' | LC_ALL=C sort
}

fetch() {
  mkdir -p "$corpus_dir"
  while IFS=$'\t' read -r name url commit <&3; do
    dir=$corpus_dir/$name
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

check() {
  [ -x "$grounds" ] || cargo build --release --quiet --manifest-path "$repo/Cargo.toml" -p grounds-cli
  cargo build --release --quiet --manifest-path "$repo/Cargo.toml" -p grounds-syntax --example errors
  errors=$repo/target/release/examples/errors
  rm -rf "$out/current" && mkdir -p "$out/current"
  failed=0

  while IFS=$'\t' read -r name _ commit <&3; do
    dir=$corpus_dir/$name
    if [ ! -d "$dir" ]; then echo "$name: missing (run fetch)"; failed=1; continue; fi
    head=$(git -C "$dir" rev-parse HEAD)
    [ "$head" = "$commit" ] || echo "$name: at ${head:0:12}, pinned ${commit:0:12}"

    findings "$dir" > "$out/current/$name.txt"
    hard=$(awk -v rules=",$hard_rules," 'index(rules, "," $2 ",")' "$out/current/$name.txt")
    parse=$(cd "$dir" && find . -name '*.java' -not -path '*/.grounds/*' -print0 | xargs -0 "$errors")
    printf '%-28s %6d findings' "$name" "$(wc -l < "$out/current/$name.txt")"
    if [ -n "$hard" ] || [ -n "$parse" ]; then
      echo "  FAIL"; failed=1
      [ -n "$hard" ] && sed 's/^/    /' <<< "$hard" | head -20
      [ -n "$parse" ] && sed 's/^/    parse: /' <<< "$parse" | head -20
    else
      echo
    fi
  done 3< "$projects"

  # Planted bugs: apply the patch to a copy and expect exactly the listed findings.
  for patch in "$repo"/corpus/planted/*.patch; do
    name=$(basename "$patch" .patch)
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

  if [ -d "$out/baseline" ]; then
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
  check) check ;;
  baseline) rm -rf "$out/baseline" && cp -R "$out/current" "$out/baseline" && echo "baseline updated" ;;
  *) echo "usage: $0 fetch|check|baseline" >&2; exit 2 ;;
esac
