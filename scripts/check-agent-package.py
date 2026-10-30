#!/usr/bin/env python3
"""Validate the repository skill and plugin manifests."""
import json
from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent
REFERENCES = ("CLI", "RECOVERY", "SAFETY", "NATIVE", "RUNS", "FILES", "SESSIONS", "MCP")


def fail(message: str) -> None:
    raise ValueError(f"check-agent-package: {message}")


def main() -> int:
    required = ["plugin.json", ".codex-plugin/plugin.json", "skills/rhost/SKILL.md"]
    required += [f"skills/rhost/references/{name}.md" for name in REFERENCES]
    for name in required:
        if not (ROOT / name).is_file():
            fail(f"missing {name}")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    for name in ("plugin.json", ".codex-plugin/plugin.json"):
        manifest = json.loads((ROOT / name).read_text())
        if manifest.get("name") != "rhost":
            fail(f"{name} name must be rhost")
        if manifest.get("version") != version:
            fail(f"{name} version must match Cargo.toml")
    if json.loads((ROOT / ".codex-plugin/plugin.json").read_text()).get("skills") != "./skills/":
        fail("Codex manifest must expose ./skills/")
    skill = (ROOT / "skills/rhost/SKILL.md").read_text()
    if "name: rhost" not in skill:
        fail("SKILL.md has no rhost name")
    for name in REFERENCES:
        if f"references/{name}.md" not in skill:
            fail(f"SKILL.md does not route to {name}.md")
    # References are navigated on demand. Validate their portable link graph,
    # without requiring mode-specific instructions to be copied into SKILL.md.
    skill_root=(ROOT/"skills/rhost").resolve()
    pending=[skill_root/"SKILL.md"]
    visited=set()
    while pending:
        path=pending.pop()
        if path in visited: continue
        visited.add(path)
        for target in re.findall(r"\[[^\]]+\]\(([^)]+)\)",path.read_text()):
            if "://" in target or target.startswith("#"): continue
            file,_,anchor=target.partition("#")
            linked=(path.parent/file).resolve()
            if not linked.is_relative_to(skill_root) or not linked.is_file():
                fail(f"broken or non-portable skill reference: {target}")
            if anchor:
                headings=re.findall(r"^#{1,6}\s+(.+)$",linked.read_text(),re.M)
                slugs={re.sub(r"[^a-z0-9_ -]","",h.lower()).replace(" ","-") for h in headings}
                if anchor not in slugs: fail(f"missing skill reference anchor: {target}")
            if linked.suffix==".md": pending.append(linked)
    for name in REFERENCES:
        if skill_root/f"references/{name}.md" not in visited:
            fail(f"unreachable skill reference: {name}.md")
    retired = re.compile(r"data\.(exit_code|timed_out|cancelled)|cleanup_confirmed|output_kind|JOB_(NOT_FOUND|STATE_UNKNOWN)|compatibility alias")
    for path in (ROOT / "skills/rhost").rglob("*"):
        if path.is_file():
            text = path.read_text()
            # Native Run streams legitimately use these paths in schema v4.
            # Keep the old exec guidance gate everywhere outside that reference.
            legacy_stream = path.name not in {"NATIVE.md", "RUNS.md"} and re.search(r"data\.(stdout|stderr)\b", text)
            if retired.search(text) or legacy_stream:
                fail(f"skill contains retired v1 wire guidance: {path.relative_to(ROOT)}")
    for text, message in (
        ("rhost version --json", "SKILL.md must tell readers to check the installed binary version"),
        ("rhost <command> --help", "SKILL.md must tell readers to check the installed binary --help"),
    ):
        if text not in skill:
            fail(message)
    if not re.search(r"symlinked|symlink", skill):
        fail("SKILL.md must warn that a symlinked skill drifts from the binary")
    transport = (ROOT / "src/transport/openssh.rs").read_text()
    match = re.search(r'^const CONTROL_PERSIST: &str = "([0-9]+m)";$', transport, re.M)
    if not match:
        fail("CONTROL_PERSIST must be a minute duration")
    persist = match.group(1)
    cli = (ROOT / "skills/rhost/references/CLI.md").read_text()
    if f"{persist[:-1]}-minute persistence" not in cli:
        fail("CLI.md persistence window differs from CONTROL_PERSIST")
    if f"ControlPersist={persist}" not in cli:
        fail("CLI.md persistence window differs from CONTROL_PERSIST")
    schema = json.loads((ROOT / "schemas/result-v4.schema.json").read_text())
    enums = []
    def walk(node: object) -> None:
        if isinstance(node, dict):
            values = node.get("enum")
            if isinstance(values, list) and "USAGE_ERROR" in values:
                enums.extend(values)
            for value in node.values():
                walk(value)
        elif isinstance(node, list):
            for value in node:
                walk(value)
    walk(schema)
    if not enums:
        fail("schema has no error.code enum")
    documented = set(re.findall(r"`([A-Z][A-Z_]{2,})`", (ROOT / "skills/rhost/references/RECOVERY.md").read_text()))
    reserved = {"UNSUPPORTED_REMOTE_OS"}
    missing = sorted(set(enums) - reserved - documented)
    unknown = sorted(documented - set(enums) - reserved - {"PATH"})
    if missing:
        fail("RECOVERY.md has no entry for: " + ", ".join(missing))
    if unknown:
        fail("RECOVERY.md names codes the schema does not define: " + ", ".join(unknown))
    print("check-agent-package: OK")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
