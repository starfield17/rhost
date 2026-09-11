#!/usr/bin/env bash
# Isolated behavioral tests for install.sh. No network or user install path is
# touched: fake uname/curl/checksum tools make each release scenario explicit.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd -P)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fake="$tmp/bin"
mkdir -p "$fake"

make_fakes() {
  cat >"$fake/uname" <<'EOF'
#!/bin/sh
case "$1" in -s) echo Linux;; -m) echo x86_64;; *) exit 2;; esac
EOF
  cat >"$fake/curl" <<'EOF'
#!/bin/sh
url=""; out=""
while [ "$#" -gt 0 ]; do
  case "$1" in -o) out=$2; shift 2;; -*) shift;; *) url=$1; shift;; esac
done
printf '%s\n' "$url" >> "$RHOST_TEST_CURL_LOG"
case "$url" in
  */releases/latest) [ "${RHOST_TEST_LATEST_FAIL:-0}" = 1 ] && exit 22; printf '%s\n' '{"tag_name":"v9.8.7"}';;
  *'/releases?per_page=1') printf '%s\n' '[{"tag_name":"v2.0.0-alpha.1"}]';;
  *.sha256) printf '%s\n' 'placeholder  asset' > "$out";;
  *) printf '%s\n' 'verified binary' > "$out";;
esac
EOF
  cat >"$fake/sha256sum" <<'EOF'
#!/bin/sh
[ "${RHOST_TEST_BAD_CHECKSUM:-0}" != 1 ]
EOF
  chmod +x "$fake/uname" "$fake/curl" "$fake/sha256sum"
}

run_install() {
  install_dir=$1
  shift
  env PATH="$fake:/usr/bin:/bin" HOME="$tmp/home" \
    RHOST_INSTALL_DIR="$install_dir" RHOST_TEST_CURL_LOG="$tmp/curl.log" \
    "$@" bash "$root/scripts/install.sh"
}

make_fakes
mkdir -p "$tmp/home"

: > "$tmp/curl.log"
run_install "$tmp/pinned" RHOST_VERSION=v1.2.3 >/dev/null
grep -q '/download/v1.2.3/rhost_1.2.3_linux_amd64$' "$tmp/curl.log"
grep -q 'verified binary' "$tmp/pinned/rhost"

: > "$tmp/curl.log"
run_install "$tmp/prerelease" RHOST_TEST_LATEST_FAIL=1 >/dev/null
grep -q '/download/v2.0.0-alpha.1/rhost_2.0.0-alpha.1_linux_amd64$' "$tmp/curl.log"

mkdir -p "$tmp/rollback"
printf '%s\n' 'existing binary' > "$tmp/rollback/rhost"
if run_install "$tmp/rollback" RHOST_VERSION=1.2.3 RHOST_TEST_BAD_CHECKSUM=1 >/dev/null 2>&1; then
  echo "checksum failure unexpectedly installed" >&2
  exit 1
fi
grep -q 'existing binary' "$tmp/rollback/rhost"

echo "test-install: OK"
