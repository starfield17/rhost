#!/usr/bin/env python3
"""Enforce source size, domain purity, dependency direction and runtime dependencies."""
import glob
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
os.chdir(ROOT)
if sys.argv[1:] not in ([], ["-v"], ["--verbose"]):
    print(f"usage: {sys.argv[0]} [-v]", file=sys.stderr)
    sys.exit(2)
VERBOSE = bool(sys.argv[1:])
HITS = 0
MAX_LINES = 500
tracked = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], capture_output=True, check=True).stdout
names = sorted({name.decode() for name in tracked.split(b"\0") if name})
files = [name for name in names if re.search(r"^(src|tests)/.*\.(rs|py)$|^scripts/.*\.py$", name)]
if not files:
    print("check-structure: no Rust or Python sources found", file=sys.stderr)
    sys.exit(2)
for name in files:
    path = Path(name)
    if not path.is_file():
        continue
    lines = len(path.read_text(encoding="utf-8").splitlines())
    if VERBOSE:
        print(f"{lines:6d}  {name}")
    if lines > MAX_LINES:
        print(f"{name}: {lines} lines exceeds the {MAX_LINES}-line budget; split it by capability")
        HITS += 1
    if name.endswith(".py"):
        try:
            compile(path.read_text(encoding="utf-8"), name, "exec")
        except SyntaxError as error:
            print(f"{name}: {error}")
            HITS += 1
installer = Path("scripts/install.sh").read_text()
marker = "exec python3 - \"$@\" <<'PY_INSTALLER'\n"
if marker in installer:
    embedded = installer.split(marker, 1)[1].split("\nPY_INSTALLER\n", 1)[0]
    try:
        compile(embedded, "scripts/install.sh:Python", "exec")
    except SyntaxError as error:
        print(f"scripts/install.sh: {error}")
        HITS += 1

for path in glob.glob("src/domain/*.rs"):
    for number, line in enumerate(Path(path).read_text().splitlines(), 1):
        if re.search(r"use (crate|std)::(io|fs|process|net|env|time)\b|use serde\b", line):
            print(f"{path}:{number}:{line}\n  (domain must not reach outside itself)")
            HITS += 1
try:
    metadata = json.loads(subprocess.run(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], capture_output=True, text=True, check=True).stdout)
except (OSError, subprocess.CalledProcessError, json.JSONDecodeError):
    print("check-structure: cannot read locked Cargo metadata", file=sys.stderr)
    sys.exit(2)
owned = [package for package in metadata["packages"] if package["name"] == "rhost"]
if len(owned) != 1:
    print("check-structure: expected one rhost package")
    HITS += 1
else:
    allowed = {"serde", "serde_json", "sha2", "signal-hook"}
    actual = set()
    problems = []
    for dependency in owned[0]["dependencies"]:
        name, kind, target = dependency["name"], dependency["kind"], dependency["target"]
        if kind == "dev":
            continue
        if kind is None and target is None and name in allowed:
            actual.add(name)
        else:
            problems.append(f"{name} (kind={kind}, target={target})")
    if actual != allowed:
        problems.append(f"runtime set {sorted(actual)} differs from {sorted(allowed)}")
    for problem in problems:
        print(f"check-structure: unreviewed Cargo dependency: {problem}")
        HITS += 1

# The policy. A module absent from this map has no policy and fails below; a
# module present with an empty set may depend on nothing first-party at all.
ALLOWED = {
    # The shared foundation. Every capability may depend on these; none of them
    # may depend on a capability.
    "domain": set(),
    "clock": set(),
    "base64": set(),
    "random": set(),
    "shell": set(),
    "stdio": set(),
    "config": set(),
    "wire": {"domain"},
    "audit": {"cli", "clock", "config", "wire"},
    "host": {"config"},
    "signals": {"domain"},
    "transport": {"base64", "config", "domain", "random", "shell"},
    "remote": {"domain", "shell", "transport"},

    # CLI primitives: the flag/argv vocabulary every capability parses with, plus
    # the stdout sink and the shared error taxonomy they map onto a status.
    "cli": {"remote", "stdio", "wire"},

    # Capabilities. A capability may reach the shared foundation and the CLI
    # primitives, and must not depend on a sibling capability; the one declared
    # exception is doctor -> connection, which reports the shared master DTO.
    "exec": {"audit", "cli", "domain", "remote", "shell", "signals", "stdio",
             "transport", "wire"},
    "hosts": {"cli", "host", "wire"},
    "connection": {"audit", "cli", "transport", "wire"},
    "doctor": {"audit", "cli", "connection", "domain", "remote", "transport", "wire"},
    "files": {"audit", "base64", "cli", "config", "domain", "remote", "shell", "transport",
             "wire"},
    "session": {"audit", "base64", "cli", "domain", "random", "remote", "shell",
                "transport", "wire"},
    "tunnel": {"audit", "cli", "config", "transport", "wire"},

    # The composition root may depend on every capability; it holds no behavior.
    "dispatch": {"audit", "cli", "connection", "doctor", "exec", "files", "hosts",
                 "session", "signals", "transport", "tunnel", "wire"},
    "main": {"dispatch", "signals", "transport"},
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

if VERBOSE:
    print("\ndependency graph (source):")
    for h in sorted(ALLOWED):
        print(f"  {h:10s} -> {' '.join(sorted(graph.get(h, ())))}")
    print("dependency graph (policy):")
    for h in sorted(ALLOWED):
        print(f"  {h:10s} -> {' '.join(sorted(ALLOWED[h]))}")
    print()


for directory in sorted(Path("src").iterdir()):
    if not directory.is_dir():
        continue
    if not (directory / "mod.rs").is_file() and not (Path("src") / f"{directory.name}.rs").is_file():
        print(f"{directory}: a module directory needs a mod.rs naming what it exports")
        HITS += 1
if HITS:
    print("\nFound structure violations.\n\nThe reading budget, dependency policy and purity rule live in\nscripts/check-structure.py; src/AGENTS.md describes the modules they guard.", file=sys.stderr)
    sys.exit(1)
print(f"check-structure: OK ({len(files)} source files, budget {MAX_LINES} lines)")
