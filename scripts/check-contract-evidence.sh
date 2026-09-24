#!/usr/bin/env bash
# Keep the standalone gate and its verbose mode; the history index and contract
# ledger share one Rust test-pointer validator.
set -euo pipefail
cd "$(dirname "$0")/.."

case "${1:-}" in
  "") cargo test --locked --test contract_evidence --quiet ;;
  -v|--verbose) CONTRACT_EVIDENCE_VERBOSE=1 cargo test --locked --test contract_evidence -- --nocapture ;;
  *) echo "usage: $0 [-v]" >&2; exit 2 ;;
esac
