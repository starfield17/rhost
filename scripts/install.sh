#!/usr/bin/env bash
# Install the latest rhost release binary into a directory on your PATH.
#
#   curl -fsSL https://raw.githubusercontent.com/starfield17/rhost/main/scripts/install.sh | bash
#
# The download is verified against the release's SHA-256 file before anything is
# replaced, and an existing binary is left in place if verification fails.
#
# Override the install directory with RHOST_INSTALL_DIR (default: ~/.local/bin).
# Pin an exact version with RHOST_VERSION; without it the newest stable release
# is installed, or the newest release of any kind while every release is a
# prerelease.
set -euo pipefail

repo="starfield17/rhost"
install_dir="${RHOST_INSTALL_DIR:-$HOME/.local/bin}"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$os" in
  darwin) os=darwin ;;
  linux)  os=linux ;;
  *) echo "unsupported OS: $os" >&2; exit 1 ;;
esac

arch="$(uname -m)"
case "$arch" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64)  arch=amd64 ;;
  *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
esac

# The tag of the first release an endpoint reports, with the leading `v` removed.
# A missing answer is not an error here: `releases/latest` 404s while every
# release is a prerelease, which is exactly the state a project is in before its
# first stable one, and the fallback below is the answer to that.
latest_tag() {
  curl -fsSL "$1" 2>/dev/null \
    | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -1 || true
}

version="${RHOST_VERSION:-}"
if [ -z "$version" ]; then
  version="$(latest_tag "https://api.github.com/repos/${repo}/releases/latest")"
fi
if [ -z "$version" ]; then
  version="$(latest_tag "https://api.github.com/repos/${repo}/releases?per_page=1")"
fi
[ -n "$version" ] || { echo "could not determine the newest version" >&2; exit 1; }

asset="rhost_${version}_${os}_${arch}"
url="https://github.com/${repo}/releases/download/v${version}/${asset}"

mkdir -p "$install_dir"
echo "downloading rhost ${version} for ${os}/${arch} ..."
# Everything lands in a temporary directory inside the install dir, so the rename
# that publishes the binary is within one filesystem (atomic) and an existing
# install is still there untouched if any step before it fails.
install_tmp="$(mktemp -d "$install_dir/.rhost-install.XXXXXX")"
trap 'rm -f "$install_tmp/$asset" "$install_tmp/$asset.sha256"; rmdir "$install_tmp"' EXIT
curl -fsSL "$url" -o "$install_tmp/$asset"
if ! curl -fsSL "$url.sha256" -o "$install_tmp/$asset.sha256"; then
  echo "no SHA-256 file for this release; refusing to install an unverified binary" >&2
  exit 1
fi
# Two spellings of the same check: `sha256sum` is coreutils and is on most Linux
# systems, `shasum` is what macOS ships. The digest format is identical.
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$install_tmp" && sha256sum -c "$asset.sha256" >/dev/null)
elif command -v shasum >/dev/null 2>&1; then
  (cd "$install_tmp" && shasum -a 256 -c "$asset.sha256" >/dev/null)
else
  echo "need sha256sum or shasum to verify the download" >&2
  exit 1
fi
chmod +x "$install_tmp/$asset"
mv "$install_tmp/$asset" "$install_dir/rhost"

echo "installed $install_dir/rhost"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "note: $install_dir is not on your PATH" ;;
esac
