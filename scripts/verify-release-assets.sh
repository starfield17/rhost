#!/bin/sh
# Check the full release inventory and bind each sidecar to its own asset.
set -eu
version=${1:?set the Cargo release version}
cd "${2:?set the release artifact directory}"

if [ "$(find . -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')" != 12 ]; then
    echo 'expected four client and two Linux helper binaries with SHA-256 sidecars' >&2
    exit 1
fi

for asset in "rhost_${version}_darwin_amd64" "rhost_${version}_darwin_arm64" \
    "rhost_${version}_linux_amd64" "rhost_${version}_linux_arm64" \
    "rhost-helper_${version}_linux-x86_64" "rhost-helper_${version}_linux-aarch64"; do
    if [ ! -f "$asset" ] || [ -L "$asset" ] || [ ! -s "$asset" ] ||
        [ ! -f "$asset.sha256" ] || [ -L "$asset.sha256" ]; then
        echo "missing or invalid release files for $asset" >&2
        exit 1
    fi
    expected=$(shasum -a 256 -- "$asset")
    if ! printf '%s\n' "$expected" | cmp -s - "$asset.sha256"; then
        echo "SHA-256 verification failed for $asset" >&2
        exit 1
    fi
done
