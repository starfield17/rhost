#!/bin/sh
# Build a native C99 helper with the digest of its embedded sources.
set -eu
cd "$(dirname "$0")/.."
out=${1:-target/helper/rhost-helper}
cc=${CC:-cc}
mkdir -p "$(dirname "$out")"
digest=$(LC_ALL=C sh scripts/helper-source-digest.sh)
temporary=$(mktemp "${out}.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM
set -- -std=c99 -Wall -Wextra -Werror -Wno-misleading-indentation
if [ "${RHOST_HELPER_SANITIZE:-0}" = 1 ]; then
    set -- "$@" -O1 -g -fsanitize=address,undefined -fno-omit-frame-pointer
else
    set -- "$@" -Os
fi
if [ "${RHOST_HELPER_STATIC:-0}" = 1 ]; then
    set -- "$@" -static
fi
"$cc" "$@" "-DRHOST_SOURCE_DIGEST=\"$digest\"" helper/*.c -o "$temporary"
chmod 700 "$temporary"
mv "$temporary" "$out"
printf '%s\n' "$out"
