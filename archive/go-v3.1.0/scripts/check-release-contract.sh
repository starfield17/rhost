#!/usr/bin/env bash
# check-release-contract.sh — the release targets have three readers, and they
# must agree before a tag exists:
#
#	Makefile     builds `rhost_<version>_<goos>_<goarch>` for RELEASE_TARGETS
#	release.yml  re-executes each asset on a runner of that platform/architecture
#	install.sh   maps `uname -s`/`uname -m` onto the same four asset names
#
# A drift between them publishes an artifact nobody can install, or installs one
# nobody built, and neither shows up until the release is already public. This
# check is what makes the four-target set a contract instead of three copies.
#
# The expected set is written out here on purpose: changing which platforms ship
# is a decision, and the decision has to be stated in every reader.
#
# Exit status: 0 consistent, 1 drift, 2 usage/environment error.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() {
	printf 'check-release-contract: %s\n' "$1" >&2
	exit 1
}

for required in VERSION plugin.json Makefile .github/workflows/release.yml scripts/install.sh; do
	[ -f "$required" ] || fail "missing $required; run from a checkout of rhost"
done

# VERSION is the shared CLI/skill release identity. Rehearsal builds may override
# the binary version, but a published tag must match the checked-in manifest.
release_version="$(tr -d '\n' < VERSION)"
printf '%s' "$release_version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' \
	|| fail "VERSION must contain MAJOR.MINOR.PATCH[-prerelease]"
plugin_version="$(sed -n 's/^[[:space:]]*"version": "\([^"]*\)",*$/\1/p' plugin.json)"
[ "$plugin_version" = "$release_version" ] || fail "plugin.json version must match VERSION"
grep -Fq 'VERSION ?= $(shell cat VERSION)' Makefile \
	|| fail "Makefile must default to VERSION"
grep -Fq 'if [ "$version" != "$(tr -d '\''\n'\'' < VERSION)" ]; then' .github/workflows/release.yml \
	|| fail "release.yml must check the release tag against VERSION"

expected_targets="darwin/amd64
darwin/arm64
linux/amd64
linux/arm64"

# Each target is verified by executing it, so the runner must be that platform
# and architecture: a cross-compiled binary that never ran is not evidence.
expected_runners="darwin/amd64 macos-15-intel
darwin/arm64 macos-15
linux/amd64 ubuntu-24.04
linux/arm64 ubuntu-24.04-arm"

# --- Makefile: the target list ------------------------------------------------

makefile_targets="$(
	sed -n 's/^RELEASE_TARGETS *:=[[:space:]]*//p' Makefile \
		| tr -s '[:space:]' '\n' | grep -v '^$' | sort
)"
[ -n "$makefile_targets" ] || fail "Makefile: no RELEASE_TARGETS list found"
if [ "$makefile_targets" != "$expected_targets" ]; then
	fail "Makefile: RELEASE_TARGETS is not the four shipping targets:
$(printf '%s\n' "$makefile_targets" | sed 's/^/  /')"
fi

grep -Fq 'ARTIFACT_VERSION := $(VERSION:v%=%)' Makefile \
	|| fail "Makefile: the artifact version must be \$(VERSION) with the leading v stripped"
grep -Fq 'rhost_$(ARTIFACT_VERSION)_' Makefile \
	|| fail "Makefile: dist must name assets rhost_<version>_<goos>_<goarch>"

# --- release.yml: where each artifact is executed -----------------------------

# Matrix rows are one `- runner:` plus `goos:`/`goarch:` lines, so this reads
# them without a YAML parser. Four rows anywhere in the file are four rows too
# many, which is the safe direction to fail in.
workflow_runners="$(
	awk '
		/^[[:space:]]*-[[:space:]]*runner:/ { runner = $3; goos = ""; goarch = ""; next }
		/^[[:space:]]*goos:/   { if (runner != "") goos = $2 }
		/^[[:space:]]*goarch:/ { if (runner != "") goarch = $2 }
		runner != "" && goos != "" && goarch != "" {
			print goos "/" goarch " " runner
			runner = ""
		}
	' .github/workflows/release.yml | sort
)"
if [ "$workflow_runners" != "$expected_runners" ]; then
	fail "release.yml: the native runners are not one per target:
$(printf '%s\n' "$workflow_runners" | sed 's/^/  /')"
fi

# The publish job recomputes the expected asset names from RELEASE_TARGETS
# rather than trusting the artifact it downloaded.
grep -Fq 'rhost_${VERSION}_${goos}_${goarch}' .github/workflows/release.yml \
	|| fail "release.yml: publish must recompute expected asset names from RELEASE_TARGETS"

# --- install.sh: which asset each uname lands on ------------------------------

install_os="$(
	sed -n 's/^[[:space:]]*\([a-z0-9|]*\))[[:space:]]*os=\([a-z0-9]*\)[[:space:]]*;;.*/\1=\2/p' scripts/install.sh \
		| sort
)"
expected_os="darwin=darwin
linux=linux"
[ "$install_os" = "$expected_os" ] \
	|| fail "install.sh: the OS mapping is not darwin->darwin and linux->linux:
$(printf '%s\n' "${install_os:-  (none)}" | sed 's/^/  /')"

install_arch="$(
	sed -n 's/^[[:space:]]*\([a-z0-9_|]*\))[[:space:]]*arch=\([a-z0-9]*\)[[:space:]]*;;.*/\1=\2/p' scripts/install.sh \
		| sort
)"
expected_arch="arm64|aarch64=arm64
x86_64|amd64=amd64"
[ "$install_arch" = "$expected_arch" ] \
	|| fail "install.sh: the uname -m mapping no longer covers macOS and Linux:
$(printf '%s\n' "${install_arch:-  (none)}" | sed 's/^/  /')"

grep -Fq 'asset="rhost_${version}_${os}_${arch}"' scripts/install.sh \
	|| fail "install.sh: the asset name must be rhost_<version>_<os>_<arch>"
# The tag keeps its `v` and the asset does not: `/v${version}/` above a
# `rhost_${version}_...` asset is the whole relationship.
grep -Fq 'releases/download/v${version}/${asset}' scripts/install.sh \
	|| fail "install.sh: the download URL must be .../download/v<version>/<asset>"

echo "check-release-contract: OK (4 targets agree across Makefile, release workflow, install.sh)"
