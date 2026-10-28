#!/bin/sh
# Canonical source identity; compiler/configuration provenance is separate.
set -eu
export LC_ALL=C
cd "$(dirname "$0")/.."
{
    for source in helper/*.[ch]; do
        printf '%s\000' "$source"
        cat "$source"
        printf '\000'
    done
} | shasum -a 256 | cut -d ' ' -f1
