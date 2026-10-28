#!/bin/sh
# Local-only C build; verification jobs consume RHOST_HELPER_BIN instead.
set -eu
if [ -n "${CI:-}" ]; then
    echo 'helper compilation is local-only; supply RHOST_HELPER_BIN in CI' >&2
    exit 2
fi
cd "$(dirname "$0")/.."
out=${1:-target/helper/rhost-helper}
cc=${CC:-cc}
mkdir -p "$(dirname "$out")"
digest=$(LC_ALL=C sh scripts/helper-source-digest.sh)
temporary=$(mktemp "${out}.XXXXXX")
trap 'rm -f "$temporary"' EXIT HUP INT TERM
if [ "${RHOST_HELPER_SANITIZE:-0}" = 1 ]; then
    "$cc" -std=c99 -Wall -Wextra -Werror -O1 -g -fsanitize=address,undefined \
        -fno-omit-frame-pointer "-DRHOST_SOURCE_DIGEST=\"$digest\"" helper/*.c -o "$temporary"
else
    "$cc" -std=c99 -Wall -Wextra -Werror -Os \
        "-DRHOST_SOURCE_DIGEST=\"$digest\"" helper/*.c -o "$temporary"
fi
chmod 700 "$temporary"
mv "$temporary" "$out"
printf '%s\n' "$out"
