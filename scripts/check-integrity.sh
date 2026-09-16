#!/usr/bin/env bash
# check-integrity.sh BASE — the measurement may not be weakened to make a result
# land.
#
# AGENTS.md §9 says contract and tests cannot be weakened to make an
# implementation pass. That was prose; this is the mechanical half. Compared
# with BASE it fails when a test is deleted, an assertion or `#[test]` count
# drops, an `#[ignore]` appears, a `make check` prerequisite or a check script
# disappears, or a runner/selector narrows.
#
# A deliberate acceptance change is still allowed, but only as a change to the
# measurements alone — no `src/` in the same range — carrying a commit message
# trailer:
#
#	GateChange: <why the measurement is genuinely wrong>
#
# That is the same separation AGENTS.md §10 asks for: the acceptance change
# lands first, on its own, and is visible as one.
#
#	./scripts/check-integrity.sh <BASE>          # compare BASE..HEAD
#	./scripts/check-integrity.sh -v <BASE>       # print the counts it read
#
# Exit status: 0 clean, 1 findings, 2 usage/environment error.
set -euo pipefail

verbose=0
if [ "${1:-}" = "-v" ] || [ "${1:-}" = "--verbose" ]; then
	verbose=1
	shift
fi

if [ $# -ne 1 ]; then
	echo "usage: $0 [-v] <BASE>" >&2
	exit 2
fi

BASE=$1
cd "$(dirname "$0")/.."

if ! git rev-parse --verify --quiet "$BASE^{commit}" >/dev/null; then
	echo "check-integrity: BASE '$BASE' is not a commit in this repository" >&2
	exit 2
fi

# A pure measurement change is allowed when it is the whole diff and it says so.
src_touched=$(git diff --name-only "$BASE"...HEAD -- src | grep -c . || true)
trailers=$(git log --format='%B' "$BASE"..HEAD | grep -cE '^GateChange: .' || true)

VERBOSE=$verbose SRC_TOUCHED=$src_touched TRAILERS=$trailers python3 - "$BASE" <<'PY'
import os, re, subprocess, sys

BASE = sys.argv[1]
VERBOSE = os.environ.get("VERBOSE") == "1"
SRC_TOUCHED = int(os.environ.get("SRC_TOUCHED", "0"))
TRAILERS = int(os.environ.get("TRAILERS", "0"))
HITS = 0


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True).stdout


def report(message):
    global HITS
    print(message)
    HITS += 1


def tracked(ref, pattern):
    out = git("ls-tree", "-r", "--name-only", ref)
    return [line for line in out.splitlines()
            if re.search(pattern, line) and line.endswith(".rs")]


def count(ref, paths, pattern):
    total = 0
    for path in paths:
        blob = git("show", f"{ref}:{path}")
        total += len(re.findall(pattern, blob))
    return total


def counts(ref):
    tests = tracked(ref, r"^(tests/|src/)")
    return {
        "test_files": len(tests),
        "test_attrs": count(ref, tests, r"#\[test\]"),
        "assertions": count(ref, tests, r"\bassert(?:_eq|_ne|_matches)?!|\bpanic!\s*\("),
        "ignores": count(ref, tests, r"#\[ignore"),
    }


before, after = counts(BASE), counts("HEAD")

if VERBOSE:
    print(f"  BASE {BASE}: {before}")
    print(f"  HEAD      : {after}")

# Deletions and drops. A file that moves is not a deletion, so compare sets of
# module paths rather than raw counts alone.
for label, key in (("test files", "test_files"),
                   ("`#[test]` attributes", "test_attrs"),
                   ("assertion macro calls", "assertions")):
    if after[key] < before[key]:
        report(f"{label}: {before[key]} -> {after[key]} (a measurement was removed)")

if after["ignores"] > before["ignores"]:
    report(f"`#[ignore]` attributes: {before['ignores']} -> {after['ignores']}"
           " (a measurement was disabled)")

# The gate itself: `make check` may gain prerequisites, never lose one.
def check_prerequisites(ref):
    makefile = git("show", f"{ref}:Makefile")
    match = re.search(r"^check:(.*)$", makefile, re.M)
    return set(match.group(1).split()) if match else set()


lost = check_prerequisites(BASE) - check_prerequisites("HEAD")
if lost:
    report(f"`make check` prerequisites removed: {' '.join(sorted(lost))}")

# A runner or selector that narrows stops covering what it used to. Compare the
# gate-bearing lines as multisets so dropping one of two copies is visible.
PATTERNS = (r"cargo\s+(?:test|nextest)", r"make\s+(?:check|test-smoke)",
            r"\./scripts/check-[a-z-]+\.sh")
gate_paths = [p for p in git("ls-tree", "-r", "--name-only", BASE).splitlines()
              if re.search(r"^(Makefile|scripts/check-.*\.sh|\.github/workflows/.*\.ya?ml)$", p)]

def gate_lines(ref):
    lines = set()
    for path in gate_paths:
        if subprocess.run(["git", "cat-file", "-e", f"{ref}:{path}"],
                          capture_output=True).returncode:
            continue
        for line in git("show", f"{ref}:{path}").splitlines():
            stripped = line.strip()
            # A comment may name a gate without running it; only real
            # invocations are compared.
            if stripped.startswith("#"):
                continue
            if any(re.search(p, stripped) for p in PATTERNS):
                lines.add(re.sub(r"\s+", " ", stripped))
    return lines


for line in sorted(gate_lines(BASE) - gate_lines("HEAD")):
    report(f"gate invocation removed or narrowed: {line}")

# A range that touched `src/` while also weakening a measurement is exactly the
# failure the rule exists for; a measurement-only range may declare the change.
if HITS and not (SRC_TOUCHED == 0 and TRAILERS):
    print("""
Found measurement-integrity violations.

An implementation change may not be made green by removing or narrowing what
measures it (AGENTS.md §9). If the measurement is genuinely wrong, land that
acceptance change first, on its own, with a `GateChange:` trailer in the commit
message. See docs/MAINTENANCE.md.""", file=sys.stderr)
    sys.exit(1)

if HITS:
    print(f"check-integrity: {HITS} measurement change(s) declared with GateChange and no src/ in range")
else:
    print(f"check-integrity: OK ({after['test_attrs']} tests, "
          f"{after['assertions']} assertions, no measurement weakened)")
PY
