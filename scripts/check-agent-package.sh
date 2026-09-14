#!/usr/bin/env bash
# Validate the repository-distributed skill and its two plugin entry points.
set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
  printf 'check-agent-package: %s\n' "$1" >&2
  exit 1
}

for file in \
  plugin.json .codex-plugin/plugin.json skills/rhost/SKILL.md \
  skills/rhost/references/CLI.md skills/rhost/references/RECOVERY.md \
  skills/rhost/references/SAFETY.md
do
  [ -f "$file" ] || fail "missing $file"
done

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml)
[ -n "$version" ] || fail "Cargo.toml has no package version"
python3 - "$version" <<'PY'
import json
import sys

expected = sys.argv[1]
for path in ("plugin.json", ".codex-plugin/plugin.json"):
    with open(path, encoding="utf-8") as source:
        manifest = json.load(source)
    if manifest.get("name") != "rhost":
        raise SystemExit(f"check-agent-package: {path} name must be rhost")
    if manifest.get("version") != expected:
        raise SystemExit(f"check-agent-package: {path} version must match Cargo.toml")
if json.load(open(".codex-plugin/plugin.json", encoding="utf-8")).get("skills") != "./skills/":
    raise SystemExit("check-agent-package: Codex manifest must expose ./skills/")
PY

grep -Fq 'name: rhost' skills/rhost/SKILL.md || fail "SKILL.md has no rhost name"
for reference in CLI.md RECOVERY.md SAFETY.md; do
  grep -Fq "references/$reference" skills/rhost/SKILL.md \
    || fail "SKILL.md does not route to $reference"
done

if grep -R -nE 'data\.(exit_code|stdout|stderr|timed_out|cancelled)|cleanup_confirmed|output_kind|JOB_(NOT_FOUND|STATE_UNKNOWN)|compatibility alias' skills/rhost; then
  fail "skill contains retired v1 wire guidance"
fi

echo "check-agent-package: OK"
