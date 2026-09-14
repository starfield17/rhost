#!/usr/bin/env bash
# Install a released rhost binary into a directory on the local PATH.
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

latest_tag() {
  curl -fsSL "$1" 2>/dev/null \
    | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -1 || true
}

version="${RHOST_VERSION:-}"
version="${version#v}"
if [ -z "$version" ]; then
  version="$(latest_tag "https://api.github.com/repos/${repo}/releases/latest")"
fi
if [ -z "$version" ]; then
  version="$(latest_tag "https://api.github.com/repos/${repo}/releases?per_page=1")"
fi
[ -n "$version" ] || { echo "could not determine the newest version" >&2; exit 1; }
case "$version" in
  *[!0-9A-Za-z.+-]*) echo "invalid RHOST_VERSION: $version" >&2; exit 1 ;;
esac

asset="rhost_${version}_${os}_${arch}"
url="https://github.com/${repo}/releases/download/v${version}/${asset}"

mkdir -p "$install_dir"
echo "downloading rhost ${version} for ${os}/${arch} ..."
install_tmp="$(mktemp -d "$install_dir/.rhost-install.XXXXXX")"
cleanup() {
  rm -f "$install_tmp/$asset" "$install_tmp/$asset.sha256"
  rmdir "$install_tmp" 2>/dev/null || true
}
trap cleanup EXIT

curl -fsSL "$url" -o "$install_tmp/$asset"
if ! curl -fsSL "$url.sha256" -o "$install_tmp/$asset.sha256"; then
  echo "no SHA-256 file for this release; refusing to install an unverified binary" >&2
  exit 1
fi
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
