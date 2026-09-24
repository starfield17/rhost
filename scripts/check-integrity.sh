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
# measurements alone — no `src/` in that commit — carrying a commit message
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

VERBOSE=$verbose python3 - "$BASE" <<'PY'
import os, re, subprocess, sys

BASE = sys.argv[1]
VERBOSE = os.environ.get("VERBOSE") == "1"
HITS = 0
DECLARED = 0


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


# The gate itself: `make check` may gain prerequisites, never lose one.
def check_prerequisites(ref):
    makefile = git("show", f"{ref}:Makefile")
    match = re.search(r"^check:(.*)$", makefile, re.M)
    return set(match.group(1).split()) if match else set()


# A runner or selector that narrows stops covering what it used to. Compare the
# gate-bearing lines as multisets so dropping one of two copies is visible.
PATTERNS = (r"cargo\s+(?:test|nextest)", r"make\s+(?:check|test-smoke)",
            r"\./scripts/check-[a-z-]+\.sh")
def gate_lines(ref, gate_paths):
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


def findings(before_ref, after_ref):
    found = []
    before, after = counts(before_ref), counts(after_ref)
    if VERBOSE:
        print(f"  {before_ref[:10]}: {before}")
        print(f"  {after_ref[:10]}: {after}")
    for label, key in (("test files", "test_files"),
                       ("`#[test]` attributes", "test_attrs"),
                       ("assertion macro calls", "assertions")):
        if after[key] < before[key]:
            found.append(f"{label}: {before[key]} -> {after[key]}")
    if after["ignores"] > before["ignores"]:
        found.append(f"`#[ignore]` attributes: {before['ignores']} -> {after['ignores']}")
    lost = check_prerequisites(before_ref) - check_prerequisites(after_ref)
    if lost:
        found.append(f"`make check` prerequisites removed: {' '.join(sorted(lost))}")
    gate_paths = [p for p in git("ls-tree", "-r", "--name-only", before_ref).splitlines()
                  if re.search(r"^(Makefile|scripts/check-.*\.sh|\.github/workflows/.*\.ya?ml)$", p)]
    for line in sorted(gate_lines(before_ref, gate_paths) - gate_lines(after_ref, gate_paths)):
        found.append(f"gate invocation removed or narrowed: {line}")
    return found


# Check each actual change, so a later implementation commit does not invalidate
# a separately reviewed GateChange, and additions cannot hide an earlier drop.
# Merge commits repeat their constituent changes; their non-merge ancestors are
# checked individually. The repository uses a linear release history today.
for commit in git("rev-list", "--reverse", f"{BASE}..HEAD").splitlines():
    parents = git("rev-list", "--parents", "-n", "1", commit).split()[1:]
    if len(parents) != 1:
        continue
    change = findings(parents[0], commit)
    if not change:
        continue
    message = git("log", "-1", "--format=%B", commit)
    declared = any(re.match(r"^GateChange: .+", line) for line in message.splitlines())
    src_touched = bool(git("diff", "--name-only", parents[0], commit, "--", "src").strip())
    if declared and not src_touched:
        DECLARED += len(change)
        if VERBOSE:
            print(f"  {commit[:10]}: {len(change)} declared measurement change(s)")
    else:
        for detail in change:
            report(f"{commit[:10]}: {detail}")

if HITS:
    print("""
Found measurement-integrity violations.

An implementation change may not be made green by removing or narrowing what
measures it (AGENTS.md §9). If the measurement is genuinely wrong, land that
acceptance change first, on its own, with a `GateChange:` trailer in the commit
message. See docs/MAINTENANCE.md.""", file=sys.stderr)
    sys.exit(1)

after = counts("HEAD")
print(f"check-integrity: OK ({after['test_attrs']} tests, "
      f"{after['assertions']} assertions, {DECLARED} declared measurement change(s))")
PY
