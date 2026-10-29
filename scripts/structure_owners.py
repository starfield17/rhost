"""Capability-local adapter boundaries, composed by check-structure.py."""
from pathlib import PurePosixPath
import re

# Everything else in src is shared execution, DTOs or infrastructure. Facades
# may export adapters, but importing such an export does not hide its owner.
ADAPTER_TREES = {"cli", "dispatch", "mcp"}
ADAPTERS = {
    "main", "exec", "exec::command", "exec::run", "audit::command",
    "connection::command", "connection::run", "doctor::command", "doctor::run",
    "hosts::command", "session::command", "session::run", "session::render",
    "session::surface", "files::command", "files::run", "files::render",
    "files::surface", "files::native_cli", "runs::command", "request::cli",
    "tunnel::command", "tunnel::run", "tunnel::render",
}


def module(path):
    parts = list(PurePosixPath(path).relative_to("src").with_suffix("").parts)
    if parts[-1] in ("mod", "lib"):
        parts.pop()
    return "::".join(parts)


def adapter(name):
    return name.split("::")[0] in ADAPTER_TREES or any(
        name == owner or name.startswith(owner + "::") for owner in ADAPTERS
        if owner != "exec"
    ) or name == "exec"


def paths(text, prefix=""):
    """Expand use trees before resolving aliases and facade re-exports."""
    text = re.sub(r"\s+", "", text)
    level, start = 0, 0
    parts = []
    for i, char in enumerate(text):
        level += (char == "{") - (char == "}")
        if char == "," and level == 0:
            parts.append(text[start:i])
            start = i + 1
    parts.append(text[start:])
    for part in parts:
        if "{" in part:
            before, after = part.split("{", 1)
            yield from paths(after.rsplit("}", 1)[0], prefix + before)
        elif part:
            yield prefix + part


def absolute(path, current):
    parts = path.split("::")
    base = current.split("::") if current else []
    if parts[0] == "rmcp":
        return "mcp::sdk::" + "::".join(parts[1:])
    if parts[0] in ("crate", "rhost"):
        return "::".join(parts[1:])
    if parts[0] == "self":
        parts.pop(0)
    while parts and parts[0] == "super":
        base = base[:-1]
        parts.pop(0)
    return "::".join(base + parts)


def production(body):
    # Inline unit tests are not shared execution. String/comment stripping is
    # supplied by the parent check before this balanced module walk.
    pattern = re.compile(r"#\[cfg\(test\)\]\s*mod\s+\w+\s*\{")
    while match := pattern.search(body):
        level, end = 1, match.end()
        while level and end < len(body):
            level += (body[end] == "{") - (body[end] == "}")
            end += 1
        body = body[:match.start()] + " " + body[end:]
    return body


def violations(sources):
    exports, imports, code = {}, {}, {}
    use = re.compile(r"\b(pub(?:\([^)]*\))?\s+)?use\s+([^;]+);")
    for path, body in sources.items():
        current = module(path)
        body = production(body)
        aliases = {}
        for statement in use.finditer(body):
            # Keep `as` visible while expanding whitespace-insensitive trees.
            tree = re.sub(r"\s+as\s+", "@", statement[2])
            for entry in paths(tree):
                target, _, alias = entry.partition("@")
                target = absolute(target, current)
                alias = alias or target.split("::")[-1]
                aliases[alias] = target
                if statement[1]:
                    exports[f"{current}::{alias}"] = target
        imports[current] = aliases
        # A facade can expose its CLI adapter. Business imports still resolve
        # to that adapter through exports below; no exemption follows it.
        facade = PurePosixPath(path).name == "mod.rs"
        code[path] = use.sub(lambda m: "" if facade and m[1] else m[0], body)

    def resolve(name):
        used = set()
        while True:
            parts = name.split("::")
            for n in range(len(parts), 0, -1):
                prefix = "::".join(parts[:n])
                if prefix in exports and prefix not in used:
                    used.add(prefix)
                    name = "::".join([exports[prefix], *parts[n:]])
                    break
            else:
                return name

    problems = []
    for path, body in code.items():
        current = module(path)
        if adapter(current) or current in {"helper::tests", "session::remote::protocol::tests"}:
            continue
        targets = []
        for statement in use.finditer(body):
            for entry in paths(re.sub(r"\s+as\s+", "@", statement[2])):
                targets.append(absolute(entry.split("@")[0], current))
        for reference in re.finditer(r"\b([A-Za-z_]\w*(?:::[A-Za-z_]\w*)+)", body):
            name = reference[1]
            first, _, rest = name.partition("::")
            if first in ("crate", "rhost", "self", "super"):
                targets.append(absolute(name, current))
            elif first in imports[current]:
                targets.append(imports[current][first] + "::" + rest)
            elif first == "rmcp":
                targets.append("mcp::sdk")
        for target in sorted(set(map(resolve, targets))):
            if adapter(target) or target.endswith("::*"):
                problems.append(f"{path}: shared execution reaches adapter `{target}`")
    return problems


def self_test():
    base = {"src/session/mod.rs": "pub use command::Request;", "src/session/command.rs": ""}
    cases = [
        ("use crate::cli::Sink;", True),
        ("use crate::{cli as output};", True),
        ("use crate::session::Request;", True),
        ("use crate::session as cap; fn f() {cap::Request::new();}", True),
        ("use super::{command as input, dto};", True),
        ("use rmcp as sdk;", True),
        ("use crate::remote::Error;", False),
        ("use super::{dto, ops};", False),
        ("#[cfg(test)] mod tests {use super::*;}", False),
    ]
    for body, refused in cases:
        found = violations({**base, "src/session/application.rs": body})
        if bool(found) != refused:
            raise ValueError(f"adapter ownership self-test: {body!r}: {found}")
