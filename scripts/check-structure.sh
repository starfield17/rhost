#!/usr/bin/env bash
# check-structure.sh — keep the source shape from rotting.
#
# Two rules, both mechanical. They exist because an agent can only change one
# part of a repository safely if that part is small enough to read and if the
# direction of a dependency is a fact rather than a convention.
#
#   1. Size: no tracked source or test file is longer than the reading budget.
#      A file that outgrows it is doing more than one job; split it by capability.
#   2. Direction: `domain` knows nothing, `fileops` may not know the use-cases,
#      and nothing below the CLI may know the CLI.
#
#	./scripts/check-structure.sh          # check
#	./scripts/check-structure.sh -v       # list every file and its size
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

# One file, one job. The budget is a ceiling on reading, not a target: every
# file here is under it, and crossing it is a signal that a capability has grown
# a second reason to change.
MAX_LINES=500

files=$(git ls-files --cached --others --exclude-standard \
	| grep -E '^(src|tests)/.*\.rs$' | sort -u || true)
if [ -z "$files" ]; then
	echo "check-structure: no Rust sources found" >&2
	exit 2
fi

hits=0

while IFS= read -r f; do
	[ -n "$f" ] || continue
	lines=$(wc -l <"$f" | tr -d ' ')
	if [ "$verbose" = 1 ]; then
		printf '%6s  %s\n' "$lines" "$f"
	fi
	if [ "$lines" -gt "$MAX_LINES" ]; then
		echo "$f: $lines lines exceeds the $MAX_LINES-line budget; split it by capability"
		hits=$((hits + 1))
	fi
done <<EOF
$files
EOF

# Dependency direction. A rule is only real if it can fail, so each one is a
# pattern that must not appear in the files listed after it.
check_forbidden() {
	label=$1
	pattern=$2
	shift 2
	# shellcheck disable=SC2086
	found=$(grep -HnE "$pattern" -- "$@" 2>/dev/null || true)
	if [ -n "$found" ]; then
		printf '%s\n  (%s)\n' "$found" "$label"
		hits=$((hits + 1))
	fi
}

# `domain` is pure: values, transitions and evidence, never I/O or a wire format.
check_forbidden "domain must not reach outside itself" \
	'use (crate|std)::(io|fs|process|net|env|time)::|use crate::(app|cli|output|transport|fileops)|use serde' \
	src/domain/*.rs

# `fileops` runs local tools; it sits below the use-cases that decide *when* to.
check_forbidden "fileops must not know the use-cases" \
	'use crate::(app|cli|output)' \
	src/fileops/*.rs

# The CLI is the only layer that may know the CLI.
check_forbidden "lower layers must not know the CLI" \
	'use crate::cli' \
	src/app/*.rs src/domain/*.rs src/fileops/*.rs src/output/*.rs src/transport/*.rs

# Every module directory names what it exports in one file. A module may also be
# `src/name.rs` plus `src/name/` submodules; then the file is the surface.
for dir in src/*/; do
	[ -d "$dir" ] || continue
	name=${dir#src/}
	name=${name%/}
	if [ ! -f "${dir}mod.rs" ] && [ ! -f "src/${name}.rs" ]; then
		echo "${dir}: a module directory needs a mod.rs naming what it exports"
		hits=$((hits + 1))
	fi
done

if [ "$hits" -gt 0 ]; then
	cat >&2 <<'EOF'

Found structure violations.

The reading budget and the dependency directions are described at the top of
this script and in src/AGENTS.md. Split a file by capability (one reason to
change), or move the dependency to the layer that owns it.
EOF
	exit 1
fi

count=$(printf '%s\n' "$files" | grep -c .)
echo "check-structure: OK ($count source files, budget $MAX_LINES lines)"
