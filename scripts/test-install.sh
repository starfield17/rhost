#!/usr/bin/env bash
# Hermetic installer tests: fake release and platform tools keep the network and
# the user's installation directories out of the test.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd -P)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fake="$tmp/bin"
mkdir -p "$fake" "$tmp/home"

make_fakes() {
  cat >"$fake/uname" <<'EOF'
#!/bin/sh
case "$1" in -s) printf '%s\n' "$RHOST_TEST_UNAME_S";; -m) printf '%s\n' "$RHOST_TEST_UNAME_M";; *) exit 2;; esac
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
  *.sha256) checksum_asset=${url##*/}; checksum_asset=${checksum_asset%.sha256}; printf '%s  %s\n' 'placeholder' "$checksum_asset" > "$out";;
  *) printf '%s\n' 'verified binary' > "$out";;
esac
EOF
cat >"$fake/sha256sum" <<'EOF'
#!/bin/sh
[ "${RHOST_TEST_BAD_CHECKSUM:-0}" != 1 ] || exit 1
[ "$1" = -c ] || exit 2
file=$2
asset=$(sed -n 's/^[^ ]*  //p' "$file")
[ -n "$asset" ] && [ -f "$asset" ]
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
for platform in \
  'Darwin x86_64 darwin_amd64' \
  'Darwin arm64 darwin_arm64' \
  'Linux x86_64 linux_amd64' \
  'Linux aarch64 linux_arm64'
do
  set -- $platform
  : > "$tmp/curl.log"
  run_install "$tmp/$3" RHOST_VERSION=v1.2.3 \
    RHOST_TEST_UNAME_S="$1" RHOST_TEST_UNAME_M="$2" >/dev/null
  grep -q "/download/v1.2.3/rhost_1.2.3_$3$" "$tmp/curl.log"
  grep -q 'verified binary' "$tmp/$3/rhost"
done

: > "$tmp/curl.log"
run_install "$tmp/prerelease" RHOST_TEST_LATEST_FAIL=1 \
  RHOST_TEST_UNAME_S=Linux RHOST_TEST_UNAME_M=x86_64 >/dev/null
grep -q '/download/v2.0.0-alpha.1/rhost_2.0.0-alpha.1_linux_amd64$' "$tmp/curl.log"

mkdir -p "$tmp/rollback"
printf '%s\n' 'existing binary' > "$tmp/rollback/rhost"
if run_install "$tmp/rollback" RHOST_VERSION=1.2.3 RHOST_TEST_BAD_CHECKSUM=1 \
  RHOST_TEST_UNAME_S=Linux RHOST_TEST_UNAME_M=x86_64 >/dev/null 2>&1; then
  echo "checksum failure unexpectedly installed" >&2
  exit 1
fi
grep -q 'existing binary' "$tmp/rollback/rhost"

if run_install "$tmp/unsupported" RHOST_VERSION=1.2.3 \
  RHOST_TEST_UNAME_S=Unsupported RHOST_TEST_UNAME_M=x86_64 >/dev/null 2>&1; then
  echo "unsupported OS unexpectedly installed" >&2
  exit 1
fi

echo "test-install: OK"
