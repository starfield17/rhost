#!/usr/bin/env bash
# Prove that the Rust-only release gate accounts for every frozen live scenario.
set -euo pipefail

cd "$(dirname "$0")/.."
map=tests/live/legacy-map.tsv
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

sed -n '/^[^#]/s/[[:space:]].*$//p' "$map" | sort -u > "$tmp/mapped"
find archive/conformance-v1/conformance -type f -name '*.go' \
  -exec grep -hEo '^func (TestLive[A-Za-z0-9_]+|TestSmokeLive)' {} + \
  | sed 's/.*func //' | sort -u > "$tmp/frozen"

if ! diff -u "$tmp/frozen" "$tmp/mapped"; then
  echo "native live coverage does not account for the frozen live corpus" >&2
  exit 1
fi

while IFS=$'\t' read -r legacy rust; do
  case "$legacy" in ''|'#'*) continue ;; esac
  if ! find tests/live tests/archive.rs tests/acceptance -type f \
    -exec grep -lF "fn ${rust}(" {} + | grep -q .; then
    echo "$legacy maps to missing Rust test $rust" >&2
    exit 1
  fi
done < "$map"

ignored=$(find tests/live_exec.rs tests/live -type f \
  -exec grep -nHF '#[ignore' {} + || true)
if [ -n "$ignored" ]; then
  printf '%s\n' "$ignored"
  echo "native live tests must be feature-gated as a suite, not individually ignored" >&2
  exit 1
fi

count=$(wc -l < "$tmp/mapped" | tr -d ' ')
echo "check-live-suite: OK ($count frozen scenarios mapped to Rust evidence)"
