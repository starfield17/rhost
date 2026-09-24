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

# The skill must tell a reader to confirm the installed binary and the skill
# agree after either changes, because a symlinked skill drifts independently.
grep -Fq 'rhost version --json' skills/rhost/SKILL.md \
  || fail "SKILL.md must tell readers to check the installed binary version"
grep -Fq 'rhost <command> --help' skills/rhost/SKILL.md \
  || fail "SKILL.md must tell readers to check the installed binary --help"
grep -Eq 'symlinked|symlink' skills/rhost/SKILL.md \
  || fail "SKILL.md must warn that a symlinked skill drifts from the binary"

# The documented connection window is a product promise. Read the transport's
# one default rather than maintaining a second numeric value in this check.
persist=$(sed -n 's/^const CONTROL_PERSIST: \&str = "\([^"]*\)";$/\1/p' src/transport/openssh.rs)
[[ "$persist" =~ ^[0-9]+m$ ]] || fail "CONTROL_PERSIST must be a minute duration"
minutes=${persist%m}
grep -Fq "${minutes}-minute persistence" skills/rhost/SKILL.md \
  || fail "SKILL.md persistence window differs from CONTROL_PERSIST"
grep -Fq "ControlPersist=${persist}" skills/rhost/references/CLI.md \
  || fail "CLI.md persistence window differs from CONTROL_PERSIST"

# RECOVERY.md claims to cover every `error.code` the binary can return. Make
# that a fact: take the authoritative set from the active schema, require a
# recovery entry for each, and refuse a code the schema does not define. A
# reserved enum value the binary never emits is listed here so the doc can say
# so honestly instead of inventing a recovery that cannot happen.
python3 - schemas/result-v2.schema.json skills/rhost/references/RECOVERY.md <<'PY'
import json
import re
import sys

schema_path, recovery_path = sys.argv[1], sys.argv[2]
# In the closed wire enum but never emitted by the current binary.
RESERVED = {"UNSUPPORTED_REMOTE_OS"}

codes = None


def walk(node):
    global codes
    if isinstance(node, dict):
        enum = node.get("enum")
        if isinstance(enum, list) and "USAGE_ERROR" in enum:
            codes = enum
        for value in node.values():
            walk(value)
    elif isinstance(node, list):
        for value in node:
            walk(value)


with open(schema_path, encoding="utf-8") as source:
    walk(json.load(source))
if not codes:
    raise SystemExit("check-agent-package: schema has no error.code enum")
codes = set(codes)

with open(recovery_path, encoding="utf-8") as source:
    recovery = source.read()
documented = set(re.findall(r"`([A-Z][A-Z_]{2,})`", recovery))

missing = sorted(code for code in codes - RESERVED if code not in documented)
unknown = sorted(documented - codes - RESERVED - {"PATH"})
if missing:
    raise SystemExit(
        "check-agent-package: RECOVERY.md has no entry for: " + ", ".join(missing)
    )
if unknown:
    raise SystemExit(
        "check-agent-package: RECOVERY.md names codes the schema does not define: "
        + ", ".join(unknown)
    )
PY

echo "check-agent-package: OK"
