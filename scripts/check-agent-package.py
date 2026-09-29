#!/usr/bin/env python3
"""Validate the repository skill and plugin manifests."""
import json
from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def fail(message: str) -> None:
    raise ValueError(f"check-agent-package: {message}")


def main() -> int:
    required = ["plugin.json", ".codex-plugin/plugin.json", "skills/rhost/SKILL.md"]
    required += [f"skills/rhost/references/{name}.md" for name in ("CLI", "RECOVERY", "SAFETY")]
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
    for name in ("CLI", "RECOVERY", "SAFETY"):
        if f"references/{name}.md" not in skill:
            fail(f"SKILL.md does not route to {name}.md")
    retired = re.compile(r"data\.(exit_code|stdout|stderr|timed_out|cancelled)|cleanup_confirmed|output_kind|JOB_(NOT_FOUND|STATE_UNKNOWN)|compatibility alias")
    for path in (ROOT / "skills/rhost").rglob("*"):
        if path.is_file() and retired.search(path.read_text()):
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
    if f"{persist[:-1]}-minute persistence" not in skill:
        fail("SKILL.md persistence window differs from CONTROL_PERSIST")
    cli = (ROOT / "skills/rhost/references/CLI.md").read_text()
    if f"ControlPersist={persist}" not in cli:
        fail("CLI.md persistence window differs from CONTROL_PERSIST")
    schema = json.loads((ROOT / "schemas/result-v2.schema.json").read_text())
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
