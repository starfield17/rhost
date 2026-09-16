#!/usr/bin/env bash
# check-structure.sh — keep the source shape from rotting.
#
# Three rules, all mechanical. They exist because an agent can only change one
# part of a repository safely if that part is small enough to read and if the
# direction of a dependency is a fact rather than a convention.
#
#   1. Size: no tracked source or test file is longer than the reading budget.
#      A file that outgrows it is doing more than one job; split it by capability.
#   2. Purity: `domain` reaches for no I/O, no environment, no wire format.
#   3. Direction: every first-party `crate::`/`rhost::` edge must appear in the
#      allowlist below, and the resulting graph must be acyclic. The allowlist is
#      the single source of truth; src/AGENTS.md describes the modules it guards
#      and does not repeat the edges.
#
#	./scripts/check-structure.sh          # check
#	./scripts/check-structure.sh -v       # also list file sizes and the graph
#
# Exit status: 0 clean, 1 findings, 2 usage/environment error.
set -euo pipefail

verbose=0
if [ "${1:-}" = "-v" ] || [ "${1:-}" = "--verbose" ]; then
	verbose=1
elif [ $# -gt 0 ]; then
	echo "usage: $0 [-v]" >&2
	exit 2
fi

cd "$(dirname "$0")/.."

# One file, one job. The budget is a ceiling on reading, not a target: every
# file here is under it, and crossing it is a signal that a capability has grown
# a second reason to change.
MAX_LINES=500

files=$(git ls-files --cached --others --exclude-standard \
	| grep -E '^(src|tests)/.*\.rs$' | sort -u || true)
if [ -z "$files" ]; then
	echo "check-structure: no Rust sources found" >&2
	exit 2
fi

hits=0

while IFS= read -r f; do
	[ -n "$f" ] || continue
	lines=$(wc -l <"$f" | tr -d ' ')
	if [ "$verbose" = 1 ]; then
		printf '%6s  %s\n' "$lines" "$f"
	fi
	if [ "$lines" -gt "$MAX_LINES" ]; then
		echo "$f: $lines lines exceeds the $MAX_LINES-line budget; split it by capability"
		hits=$((hits + 1))
	fi
done <<EOF
$files
EOF

# `domain` is pure: values, transitions and evidence, never I/O or a wire
# format. Its *crate* edges are governed by the allowlist below; this is the
# standard-library and third-party half of the same rule.
found=$(grep -HnE 'use (crate|std)::(io|fs|process|net|env|time)\b|use serde\b' src/domain/*.rs 2>/dev/null || true)
if [ -n "$found" ]; then
	printf '%s\n  (domain must not reach outside itself)\n' "$found"
	hits=$((hits + 1))
fi

# Dependency direction, from the source itself. The allowlist is the one place
# a module's permitted edges are written down; `-v` prints it next to the graph
# the source actually produced.
if ! VERBOSE="$verbose" python3 - <<'PY'
import glob, itertools, os, re, sys

HITS = 0

# The policy. A module absent from this map has no policy and fails below; a
# module present with an empty set may depend on nothing first-party at all.
ALLOWED = {
    "domain": set(),
    "clock": set(),
    "base64": set(),
    "random": set(),
    "shell": set(),
    "stdio": set(),
    "config": set(),
    "audit": {"clock"},
    "host": {"config"},
    "signals": {"domain"},
    "transport": {"base64", "config", "domain", "random", "shell"},
    "fileops": {"domain", "shell", "transport"},
    "session": {"base64", "random", "shell", "transport"},
    "tunnel": {"config", "transport"},
    "app": {"config", "domain", "fileops", "session", "shell", "transport"},
    "output": {"app", "audit", "domain", "host", "transport", "tunnel"},
    "cli": {"app", "audit", "config", "domain", "fileops", "host", "output",
            "session", "shell", "signals", "stdio", "transport", "tunnel"},
    "main": {"cli", "signals", "transport"},
}


def strip_rust(src):
    """Drop comments and literals, keep code (`crate::x` as a macro argument is
    code, not text). Handles raw strings so embedded shell does not masquerade
    as Rust."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if c == "/" and src.startswith("/*", i):
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            out.append(" ")
            continue
        m = re.match(r"(?:b?r|rb)(#*)\"", src[i:])
        if m:
            close = '"' + m.group(1)
            j = src.find(close, i + m.end())
            i = n if j < 0 else j + len(close)
            out.append('""')
            continue
        if src.startswith('b"', i):
            i += 1
        if src[i] == '"':
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            i = j
            out.append('""')
            continue
        if c == "'":
            m = re.match(r"'(?:\\.|[^'\\])'", src[i:])
            i = i + m.end() if m else i + 1
            out.append(" ")
            continue
        out.append(c)
        i += 1
    return "".join(out)


MODS = set()
for path in glob.glob("src/*"):
    name = os.path.basename(path)
    MODS.add(name if os.path.isdir(path) else name[:-3])
MODS.discard("lib")


def home(path):
    top = os.path.relpath(path, "src").replace(os.sep, "/").split("/")[0]
    return top[:-3] if top.endswith(".rs") else top


def refs(code):
    found = set()
    for m in re.finditer(r"\b(?:crate|rhost)::", code):
        rest = code[m.end():]
        idm = re.match(r"[A-Za-z_][A-Za-z_0-9]*", rest)
        if not idm:
            continue
        name, after = idm.group(0), rest[idm.end():].lstrip()
        if after.startswith("::") or after[:1] in (";", ","):
            found.add(name)
    for m in re.finditer(r"\b(?:crate|rhost)::\{([^}]*)\}", code):
        for part in m.group(1).split(","):
            tok = part.strip().split(" as ")[0].strip()
            tm = re.match(r"[A-Za-z_][A-Za-z_0-9]*", tok)
            if tm:
                found.add(tm.group(0))
    return found & MODS


graph = {}
for path in sorted(glob.glob("src/**/*.rs", recursive=True)):
    src = strip_rust(open(path, encoding="utf-8").read())
    h = home(path)
    if h == "lib":
        continue
    graph.setdefault(h, set()).update(refs(src) - {h})

# An edge the policy does not name is a boundary violation, not a missing entry.
for h in sorted(graph):
    if h not in ALLOWED:
        print(f"src/{h}: no dependency-policy entry names this module")
        HITS += 1
        continue
    for dep in sorted(graph[h] - ALLOWED[h]):
        print(f"src/{h}: depends on `{dep}`, which the policy does not allow")
        HITS += 1

# An acyclic graph is the point of a direction rule; the policy must be one too.
for label, edges in (("source", graph),
                     ("policy", {k: set(v) for k, v in ALLOWED.items()})):
    colour, stack = {}, []

    def visit(node):
        global HITS
        colour[node] = 1
        stack.append(node)
        for dep in sorted(edges.get(node, ())):
            if colour.get(dep, 0) == 1:
                loop = stack[stack.index(dep):] + [dep]
                print(f"{label} dependency cycle: {' -> '.join(loop)}")
                HITS += 1
            elif colour.get(dep, 0) == 0:
                visit(dep)
        stack.pop()
        colour[node] = 2

    for node in sorted(edges):
        if colour.get(node, 0) == 0:
            visit(node)

if os.environ.get("VERBOSE") == "1":
    print("\ndependency graph (source):")
    for h in sorted(ALLOWED):
        print(f"  {h:10s} -> {' '.join(sorted(graph.get(h, ())))}")
    print("dependency graph (policy):")
    for h in sorted(ALLOWED):
        print(f"  {h:10s} -> {' '.join(sorted(ALLOWED[h]))}")
    print()

sys.exit(1 if HITS else 0)
PY
then
	hits=$((hits + 1))
fi

# Every module directory names what it exports in one file. A module may also be
# `src/name.rs` plus `src/name/` submodules; then the file is the surface.
for dir in src/*/; do
	[ -d "$dir" ] || continue
	name=${dir#src/}
	name=${name%/}
	if [ ! -f "${dir}mod.rs" ] && [ ! -f "src/${name}.rs" ]; then
		echo "${dir}: a module directory needs a mod.rs naming what it exports"
		hits=$((hits + 1))
	fi
done

if [ "$hits" -gt 0 ]; then
	cat >&2 <<'EOF'

Found structure violations.

The reading budget, the dependency policy and the purity rule live in
scripts/check-structure.sh; src/AGENTS.md describes the modules they guard and
how to read the policy. Split a file by capability (one reason to change), or
move the dependency to the layer that owns it.
EOF
	exit 1
fi

count=$(printf '%s\n' "$files" | grep -c .)
echo "check-structure: OK ($count source files, budget $MAX_LINES lines)"
