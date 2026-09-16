#!/usr/bin/env bash
# Prove that the Rust-only release gate accounts for every frozen live scenario.
#
# The frozen live corpus is no longer read from the archived Go harness (that
# history now lives in the read-only rhost-go-old repository). The corpus list is
# `tests/live/legacy-map.tsv` itself: its left column names each frozen scenario,
# its right column names the Rust test that now carries that behavior.
set -euo pipefail

cd "$(dirname "$0")/.."
map=tests/live/legacy-map.tsv

count=$(awk -F'\t' '!/^#/ && NF && $1 != "" { n++ } END { print n+0 }' "$map")
if [ "$count" -eq 0 ]; then
  echo "check-live-suite: $map has no frozen scenarios" >&2
  exit 2
fi

# Every frozen scenario names exactly one Rust evidence symbol, and that symbol
# must exist as a test function in the active tree.
while IFS=$'\t' read -r legacy rust _; do
  case "$legacy" in ''|'#'*) continue ;; esac
  if [ -z "${rust//[[:space:]]/}" ]; then
    echo "frozen scenario $legacy has no Rust evidence symbol" >&2
    exit 1
  fi
  if ! find tests/live tests/acceptance tests/legacy_boundary.rs -type f \
    -exec grep -lF "fn ${rust}(" {} + | grep -q .; then
    echo "$legacy maps to missing Rust test $rust" >&2
    exit 1
  fi
done < "$map"

ignored=$(find tests/live_exec.rs tests/live -type f \
  -exec grep -nHF '#\[ignore' {} + || true)
if [ -n "$ignored" ]; then
  printf '%s\n' "$ignored"
  echo "native live tests must be feature-gated as a suite, not individually ignored" >&2
  exit 1
fi

echo "check-live-suite: OK ($count frozen scenarios mapped to Rust evidence)"
