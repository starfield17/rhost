#!/usr/bin/env python3
"""Reject local-only details in tracked and unignored files."""
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
HARD = re.compile(
    'mac'
    'book|mac[ _-]'
    'book|think'
    'pad|think'
    'centre|lat'
    'itude|insp'
    'iron|surface[ _-]'
    'studio|orange'
    'pi|orange[ _-]?'
    'pi|rasp'
    'berry|rasp[ _-]?'
    'pi|jet'
    'son|banana[ _-]?'
    'pi|rock[ _-]?'
    'pro|nano'
    'pi|rad'
    'xa|friendly'
    'arm|fine[ _-]?'
    'riscv|beagle'
    'bone|ge'
    'force|\\brt'
    'x[ _-]?[0-9]{3,4}\\b|rk3[0-9]{3}|s9[0-9]{2}|192\\.16'
    '8\\.[0-9]|10\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}|172\\.(1[6-9]|2[0-9]|3[01])\\.[0-9]|169\\.25'
    '4\\.|[A-Za-z0-9._%+-]+@[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}|[A-Za-z0-9_-]+\\.lo'
    'cal\\b'
)
PATH_PATTERN = re.compile(r"/home/[A-Za-z0-9._-]+/|/Users/[A-Za-z0-9._-]+/|[A-Za-z]:[\\/]Users[\\/]")
ALLOW_PATH = re.compile(r"/home/(dev|user|you|username|name|<user>)/|/Users/(dev|user|you|username|name|<user>)/|[A-Za-z]:\\Users\\<user>\\")


def main() -> int:
    if sys.argv[1:] not in ([], ["-v"], ["--verbose"]):
        print(f"usage: {sys.argv[0]} [-v]", file=sys.stderr)
        return 2
    verbose = bool(sys.argv[1:])
    result = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT, capture_output=True, check=True)
    names = sorted({name.decode() for name in result.stdout.split(b"\0") if name and name != b"AGENTS.md"})
    if not names:
        print("check-portability: no tracked files found", file=sys.stderr)
        return 2
    if verbose:
        print(f"hard pattern: {HARD.pattern}")
        print(f"path pattern: {PATH_PATTERN.pattern} (allowlist: {ALLOW_PATH.pattern})")
        print(f"files scanned: {len(names)}")
    hits = 0
    for name in names:
        path = ROOT / name
        if not path.is_file():
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            if HARD.search(line):
                print(f"{name}:{number}:{line}")
                hits += 1
            if PATH_PATTERN.search(line) and not ALLOW_PATH.search(line):
                print(f"{name}:{number}:{line}")
                hits += 1
    if hits:
        print("\nFound local-environment specifics (AGENTS.md §1).\n"
              "Replace them with generic roles: local machine, remote Linux host,\n"
              "user@example-host, ~/work/foo, /home/dev/..., gpu. A real test target belongs\n"
              "in RHOST_TEST_HOST at invocation time, or in a git-ignored path.", file=sys.stderr)
        return 1
    print(f"check-portability: OK ({len(names)} repository files, no local-environment specifics)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
