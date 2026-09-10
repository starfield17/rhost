#!/usr/bin/env bash
# Install the latest rhost release binary into a directory on your PATH.
#
#   curl -fsSL https://raw.githubusercontent.com/starfield17/rhost/main/scripts/install.sh | sh
#
# Override the install directory with RHOST_INSTALL_DIR (default: ~/.local/bin).
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

version="${RHOST_VERSION:-}"
if [ -z "$version" ]; then
  version="$(curl -fsSL "https://api.github.com/repos/${repo}/releases/latest" \
    | sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -1)"
fi
[ -n "$version" ] || { echo "could not determine latest version" >&2; exit 1; }

asset="rhost_${version}_${os}_${arch}"
url="https://github.com/${repo}/releases/download/v${version}/${asset}"

mkdir -p "$install_dir"
echo "downloading $asset ..."
curl -fsSL "$url" -o "$install_dir/rhost"
chmod +x "$install_dir/rhost"

echo "installed $install_dir/rhost"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) echo "note: $install_dir is not on your PATH" ;;
esac
