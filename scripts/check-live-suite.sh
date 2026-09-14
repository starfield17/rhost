#!/usr/bin/env bash
# Prove that the Rust-only release gate accounts for every frozen live scenario.
set -euo pipefail

cd "$(dirname "$0")/.."
map=tests/live/legacy-map.tsv
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

sed -n '/^[^#]/s/[[:space:]].*$//p' "$map" | sort -u > "$tmp/mapped"
rg -o '^func (TestLive[A-Za-z0-9_]+|TestSmokeLive)' \
  archive/conformance-v1/conformance -g '*.go' \
  | sed 's/.*func //' | sort -u > "$tmp/frozen"

if ! diff -u "$tmp/frozen" "$tmp/mapped"; then
  echo "native live coverage does not account for the frozen live corpus" >&2
  exit 1
fi

while IFS=$'\t' read -r legacy rust; do
  case "$legacy" in ''|'#'*) continue ;; esac
  if ! rg -q "fn ${rust}\\b" tests/live tests/archive.rs tests/acceptance; then
    echo "$legacy maps to missing Rust test $rust" >&2
    exit 1
  fi
done < "$map"

if rg -n '#\[ignore' tests/live_exec.rs tests/live; then
  echo "native live tests must be feature-gated as a suite, not individually ignored" >&2
  exit 1
fi

count=$(wc -l < "$tmp/mapped" | tr -d ' ')
echo "check-live-suite: OK ($count frozen scenarios mapped to Rust evidence)"
