#!/usr/bin/env bash
# check-release-contract.sh — the release workflow still enforces the contract.
#
# CONTRACT.md RELEASE-001 promises four shipping artifacts, built and executed on
# a native runner each, with their checksums verified, before publication.
# The workflow must build exactly the four shipping
# platform/runner pairs, the version taken from Cargo.toml (the sole version
# authority), the artifact named, executed and summed the way a release expects,
# no cross-compilation or write near the frozen archive, and every action pinned
# to a commit. Manual dispatch rehearses without publishing; a matching version
# tag may publish only after every native matrix job succeeds.
#
#	./scripts/check-release-contract.sh          # check
#	./scripts/check-release-contract.sh -v       # also print the pairs it read
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

workflow=.github/workflows/release.yml
manifest=Cargo.toml
installer=scripts/install.sh

for file in "$workflow" "$manifest" "$installer" plugin.json .codex-plugin/plugin.json; do
	if [ ! -f "$file" ]; then
		echo "check-release-contract: $file is missing" >&2
		exit 2
	fi
done

hits=0

report() {
	printf '%s\n' "$1"
	hits=$((hits + 1))
}

# Something the workflow must contain exactly once. A missing step would make
# the artifact something other than the shipping one.
require_once() {
	label=$1
	pattern=$2
	count=$(grep -cF "$pattern" -- "$workflow" || true)
	if [ "$count" -ne 1 ]; then
		report "expected exactly one '$label', found $count"
	fi
}

# Something the workflow must never contain: a second version authority, a
# cross-compiler, or a write into a frozen tree.
refuse() {
	label=$1
	pattern=$2
	found=$(grep -nE "$pattern" -- "$workflow" || true)
	if [ -n "$found" ]; then
		report "must not appear ($label):"
		printf '%s\n' "$found"
	fi
}

# The four shipping artifacts, in the order the matrix declares them. "Native"
# is this mapping: each artifact is built on the runner that matches its target,
# so a new pair is a deliberate contract change rather than a silent one.
expected_pairs=$(cat <<'EOF'
macos-15-intel darwin_amd64
macos-15 darwin_arm64
ubuntu-24.04 linux_amd64
ubuntu-24.04-arm linux_arm64
EOF
)
actual_pairs=$(awk '
	/^[[:space:]]*-[[:space:]]*runner:[[:space:]]*/ {
		value = $0
		sub(/^[^:]*:[[:space:]]*/, "", value)
		gsub(/[[:space:]]+$/, "", value)
		pending = value
		next
	}
	/^[[:space:]]*platform:[[:space:]]*/ {
		if (pending == "") next
		value = $0
		sub(/^[^:]*:[[:space:]]*/, "", value)
		gsub(/[[:space:]]+$/, "", value)
		print pending, value
		pending = ""
	}
' "$workflow")

if [ "$actual_pairs" != "$expected_pairs" ]; then
	report "the matrix is not the four shipping platform/runner pairs:"
	printf 'expected:\n%s\nfound:\n%s\n' "$expected_pairs" "$actual_pairs"
fi

# One build, of the one binary, from the lockfile.
require_once "locked release build" "cargo build --locked --release --bin rhost"

# The workflow proves this definition before it builds anything, so a drifted
# workflow cannot produce artifacts that look shipping-shaped.
require_once "self-check step" "./scripts/check-release-contract.sh"

# `make build-release` is the local release candidate, so it must be the same
# locked, optimized, provenance-stamped build the workflow runs. `make build`
# stays the debug build, and neither may drop the lockfile.
maketarget=$(awk '/^build-release:/ { inside = 1; next } inside && /^[^[:space:]]/ { inside = 0 } inside { print }' Makefile)
for pattern in 'cargo build --locked --release --bin rhost' 'RHOST_BUILD_COMMIT' 'RHOST_BUILD_DATE'; do
	if ! printf '%s\n' "$maketarget" | grep -Fq "$pattern"; then
		report "make build-release is missing: $pattern"
	fi
done
if printf '%s\n' "$maketarget" | grep -Fq -- '--debug'; then
	report "make build-release must not build the debug profile"
fi

# The version comes from Cargo.toml, and from nowhere else.
require_once "version read from Cargo.toml" "sed -n 's/^version = \"\\([^\"]*\\)\"/\\1/p' Cargo.toml"
refuse "a second version authority" 'plugin\.json|(/|\s)VERSION([^_A-Z]|$)'

# Plugin manifests mirror the Cargo version for distribution metadata. Cargo is
# still authoritative; drift is a failed check, never a second input to builds.
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$manifest")
for plugin in plugin.json .codex-plugin/plugin.json; do
	plugin_version=$(sed -n 's/^[[:space:]]*"version": "\([^"]*\)",*$/\1/p' "$plugin")
	if [ "$plugin_version" != "$version" ]; then
		report "$plugin version must mirror Cargo.toml"
	fi
done

# The artifact is named for the version and the platform, executed, and summed;
# the checksum is then verified, not merely written.
require_once "artifact name" 'artifact="dist/rhost_${version}_${PLATFORM}"'
require_once "artifact executes version --json" '"$artifact" version --json'
require_once "artifact version matches the manifest" 'assert d["data"]["version"] == sys.argv[1]'
require_once "artifact commit matches the checkout" 'assert d["data"]["commit"] == sys.argv[1]'
require_once "artifact build date is a UTC timestamp" 're.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z"'
require_once "artifact executes --help" '"$artifact" --help'
require_once "artifact basename selected" 'asset_name=${artifact##*/}'
require_once "basename checksum written" 'shasum -a 256 "$asset_name" > "$asset_name.sha256"'
require_once "basename checksum verified" 'shasum -a 256 -c "$asset_name.sha256"'

# The local installer must select the same four asset suffixes as the native
# matrix and verify a checksum before replacing an existing binary.
for pattern in \
	'darwin) os=darwin' \
	'linux)  os=linux' \
	'arm64|aarch64) arch=arm64' \
	'x86_64|amd64)  arch=amd64' \
	'asset="rhost_${version}_${os}_${arch}"' \
	'releases/download/v${version}/${asset}'
do
	if ! grep -Fq "$pattern" "$installer"; then
		report "install.sh is missing release mapping: $pattern"
	fi
done
if ! grep -Eq 'sha256sum|shasum' "$installer"; then
	report "install.sh does not verify SHA-256"
fi

# Publishing is one final job, restricted to a version tag and dependent on all
# four native matrix entries. Manual dispatch therefore remains a rehearsal.
require_once "version-tag trigger" '      - "v*"'
require_once "publication tag guard" "    if: startsWith(github.ref, 'refs/tags/v')"
require_once "publication waits for native artifacts" "    needs: native"
require_once "release write permission" "      contents: write"
require_once "all artifacts downloaded together" "          merge-multiple: true"
require_once "tag matches Cargo version" '          if [ "$GITHUB_REF_TYPE" = tag ]; then test "$GITHUB_REF_NAME" = "v${version}"; fi'
require_once "eight release files present" "          test \"\$(find dist -maxdepth 1 -type f -name 'rhost_*' | wc -l | tr -d ' ')\" = 8"
require_once "GitHub release publication" '          gh release create "v${version}" dist/* --verify-tag --title "rhost v${version}" --generate-notes'

publish_job=$(awk '/^  publish:/ { inside = 1 } inside { print }' "$workflow")
publish_checkouts=$(printf '%s\n' "$publish_job" |
	grep -cE '^[[:space:]]*-[[:space:]]+uses: actions/checkout@[0-9a-f]{40}$' || true)
if [ "$publish_checkouts" -ne 1 ]; then
	report "the publish job must check out the tagged repository exactly once"
fi
refuse "a second publisher" 'action-gh-release|actions/create-release|cargo publish|git tag |git push|docker push|npm publish'
refuse "an unrelated write permission" 'id-token: write|packages: write'
refuse "a cross-compiler" '\-\-target|qemu|cross build|\bzig\b'
refuse "a frozen tree" 'archive/|reference/'

triggers=$(awk '
	/^on:/ { inside = 1; next }
	inside && /^[^[:space:]#]/ { inside = 0 }
	inside { print }
' "$workflow" | grep -vE '^[[:space:]]*(#|$)' || true)
expected_triggers=$(cat <<'EOF'
  workflow_dispatch:
  push:
    tags:
      - "v*"
EOF
)
if [ "$triggers" != "$expected_triggers" ]; then
 report "the workflow triggers are not manual rehearsal plus version tags:"
	printf '%s\n' "$triggers"
fi

# Every action is pinned to a full commit, so a moved tag cannot change what the
# shipping artifacts were built from.
unpinned=$(grep -nE '^[[:space:]]*(-[[:space:]]+)?uses:' -- "$workflow" |
	grep -vE '@[0-9a-f]{40}' || true)
if [ -n "$unpinned" ]; then
	report "every action must be pinned to a 40-character commit:"
	printf '%s\n' "$unpinned"
fi

if [ "$hits" -gt 0 ]; then
	cat >&2 <<'EOF'

Found release-contract violations.

The release workflow and this check are described in docs/CONTRACT.md
(RELEASE-001) and docs/RUST_MIGRATION.md. Publication must remain downstream of
all four natively executed and checksum-verified artifacts.
EOF
	exit 1
fi

if [ "$verbose" = 1 ]; then
	printf '%s\n' "$actual_pairs"
fi
pairs=$(printf '%s\n' "$actual_pairs" | grep -c .)
echo "check-release-contract: OK ($pairs native platform/runner pairs, tag-gated publication)"
