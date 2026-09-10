#!/usr/bin/env bash
# check-portability.sh — enforce AGENTS.md §1: no local-environment specifics in
# tracked repository content.
#
# Scans tracked files for specific laptop/board models, private IP addresses,
# real-looking SSH logins, and author home paths. Documentation legitimately uses
# placeholder host names (`gpu`, `user@example-host`, `/home/dev/...`) and generic
# platform classes (`Linux`, `WSL2`, `darwin/arm64`); those are not matched.
#
#	./scripts/check-portability.sh            # scan all tracked files
#	./scripts/check-portability.sh -v         # show the pattern and file count
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

# AGENTS.md must name the banned patterns in order to forbid them; it is the
# only tracked file allowed to be skipped. Add new hardware families to the
# pattern list below as testing moves to different boxes.
# Hardware families and network identifiers: always a leak, no exceptions.
# Each literal is written as two adjacent shell strings ('mac''book'): bash
# concatenates them, so the regex is unchanged, but this file no longer contains
# the strings it forbids. Without that, tracking this script would make it fail
# its own scan.
# Hardware families and network identifiers: always a leak, no exceptions.
patterns_hard='mac''book|mac[ _-]''book|think''pad|think''centre|lat''itude|insp''iron|surface[ _-]''studio|orange''pi|orange[ _-]?''pi|rasp''berry|rasp[ _-]?''pi|jet''son|banana[ _-]?''pi|rock[ _-]?''pro|nano''pi|rad''xa|friendly''arm|fine[ _-]?''riscv|beagle''bone|rk3[0-9]{3}|s9[0-9]{2}|192\.16''8\.[0-9]|10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|172\.(1[6-9]|2[0-9]|3[01])\.[0-9]|169\.25''4\.|[A-Za-z0-9._%+-]+@[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|[A-Za-z0-9_-]+\.lo''cal\b'

# Home paths: a concrete account name is a leak, a documented placeholder is not.
patterns_path='/home/[A-Za-z0-9._-]+/|/Users/[A-Za-z0-9._-]+/|[A-Za-z]:[\\/]Users[\\/]'
allowlist='/home/(dev|user|you|username|name|<user>)/|/Users/(dev|user|you|username|name|<user>)/|[A-Za-z]:\\Users\\<user>\\'

files=$(git ls-files | grep -v '^AGENTS\.md$' || true)
if [ -z "$files" ]; then
	echo "check-portability: no tracked files found" >&2
	exit 2
fi
count=$(printf '%s\n' "$files" | grep -c .)

if [ "$verbose" = 1 ]; then
	echo "hard pattern: $patterns_hard"
	echo "path pattern: $patterns_path (allowlist: $allowlist)"
	echo "files scanned: $count"
fi

hits=0
while IFS= read -r f; do
	[ -n "$f" ] || continue
	[ -f "$f" ] || continue
	while IFS= read -r line; do
		printf '%s\n' "$line"
		hits=$((hits + 1))
	done < <(grep -HinE "$patterns_hard" -- "$f" 2>/dev/null || true)
	while IFS= read -r line; do
		if printf '%s' "$line" | grep -Eq "$allowlist"; then
			continue
		fi
		printf '%s\n' "$line"
		hits=$((hits + 1))
	done < <(grep -HinE "$patterns_path" -- "$f" 2>/dev/null || true)
done <<EOF
$files
EOF

if [ "$hits" -gt 0 ]; then
	cat >&2 <<'EOF'

Found local-environment specifics (AGENTS.md §1).
Replace them with generic roles: "local machine", "remote Linux host",
user@example-host, ~/work/foo, /home/dev/..., gpu. A real test target belongs in
RHOST_TEST_HOST at invocation time, or in a git-ignored path -- never in a
tracked file.
EOF
	exit 1
fi

echo "check-portability: OK ($count tracked files, no local-environment specifics)"
