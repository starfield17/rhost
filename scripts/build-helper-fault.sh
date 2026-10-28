#!/bin/sh
# Local instrumented companion; CI must never substitute this for runtime bytes.
set -eu
[ -z "${CI:-}" ] || { echo 'helper compilation is local-only' >&2; exit 2; }
cd "$(dirname "$0")/.."
out=${1:-target/helper/rhost-helper-fault}
cc=${CC:-cc}
digest=$(sh scripts/helper-source-digest.sh)
mkdir -p "$(dirname "$out")"
scratch=$(mktemp -d "$(dirname "$out")/.fault-build.XXXXXX")
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
set --
for source in helper/*.c; do
    object="$scratch/${source##*/}.o"
    "$cc" -std=c99 -O1 -g -Wall -Wextra -Werror -fsanitize=address,undefined -fno-omit-frame-pointer \
        -Dmalloc=rh_test_malloc -Dcalloc=rh_test_calloc -Drealloc=rh_test_realloc -Dstrdup=rh_test_strdup \
        "-DRHOST_SOURCE_DIGEST=\"$digest\"" -c "$source" -o "$object"
    set -- "$@" "$object"
done
"$cc" -std=c99 -O1 -g -Wall -Wextra -Werror -fsanitize=address,undefined \
    "$@" helper/testing/fail_alloc.c -o "$scratch/helper"
chmod 700 "$scratch/helper"
mv "$scratch/helper" "$out"
printf '%s\n' "$out"
