#!/usr/bin/env bash
# check-contract-evidence.sh — current truth comes first.
#
# docs/CONTRACT.md is the semantic ledger. Its "Executable evidence" column must
# point at current, active evidence: every contract row has to name at least one
# evidence target that exists in this tree. A row whose invariant genuinely has
# no test yet may be exempted explicitly with `[no-active-test]`, which is
# visible in the ledger rather than hidden. (Historical Go links were removed at
# v4.4.1 when that tree moved to the read-only rhost-go-old repository.)
#
#	./scripts/check-contract-evidence.sh          # check
#	./scripts/check-contract-evidence.sh -v       # also list rows and targets
#
# Exit status: 0 clean, 1 findings, 2 usage/environment error.
set -euo pipefail

verbose=0
if [ "${1:-}" = "-v" ] || [ "${1:-}" = "--verbose" ]; then
	verbose=1
elif [ $# -gt 0 ]; then
	echo "usage: $0 [-v]" >&2
	exit 2
fi

cd "$(dirname "$0")/.."
ledger=docs/CONTRACT.md
[ -f "$ledger" ] || { echo "check-contract-evidence: $ledger is missing" >&2; exit 2; }

VERBOSE="$verbose" python3 - "$ledger" <<'PY'
import os, re, sys

path = sys.argv[1]
VERBOSE = os.environ.get("VERBOSE") == "1"
HITS = 0
rows = 0

# A contract row: `| ID | behavior | json | evidence |` where ID matches the
# ledger's `<AREA>-<NNN>` convention. Prose tables and the header are skipped.
row_re = re.compile(r"^\|\s*([A-Z]+-[0-9]{3})\s*\|")
link_re = re.compile(r"\[[^\]]+\]\(([^)]+)\)")

for line in open(path, encoding="utf-8"):
    m = row_re.match(line)
    if not m:
        continue
    rows += 1
    cid = m.group(1)
    if "[no-active-test]" in line:
        if VERBOSE:
            print(f"  {cid}: exempt (no active test)")
        continue
    links = link_re.findall(line)
    active = []
    for target in links:
        target = target.split("#", 1)[0]
        if target.startswith(("http://", "https://", "mailto:")):
            continue
        # A link may name a symbol after the path (`tests/x.rs:test_name`); only
        # the path has to exist.
        m = re.match(r"^(.*\.[A-Za-z0-9_]+):([A-Za-z_][A-Za-z0-9_]*)$", target)
        path_part = m.group(1) if m else target
        # Relative to docs/, so a `../x` or a bare path both resolve from docs/.
        base = os.path.normpath(os.path.join("docs", path_part))
        exists = os.path.exists(base)
        if VERBOSE:
            print(f"  {cid}: {target} exists={exists}")
        if exists:
            active.append(target)
    if not active:
        print(f"{cid}: no evidence link that exists in this tree")
        HITS += 1

if HITS:
    print(
        "\nEvery contract row needs a current evidence link. Add a link to the\n"
        "active Rust test/source that asserts the invariant, or mark the row\n"
        "`[no-active-test]` if the invariant genuinely has no test yet.",
        file=sys.stderr,
    )
    sys.exit(1)

print(f"check-contract-evidence: OK ({rows} contract rows, current evidence present)")
PY
