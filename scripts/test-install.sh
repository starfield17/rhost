#!/usr/bin/env sh
set -eu
command -v python3 >/dev/null 2>&1 || { echo "test-install: Python 3.11+ is required" >&2; exit 2; }
python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' || { echo "test-install: Python 3.11+ is required" >&2; exit 2; }
exec python3 "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)/test-install.py" "$@"
