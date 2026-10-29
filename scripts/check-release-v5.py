#!/usr/bin/env python3
"""Additive v5 helper and two-phase promotion policy checks."""
from pathlib import Path
import re
from helper_artifact import lock

ROOT = Path(__file__).resolve().parent.parent
lock()
release = (ROOT / ".github/workflows/release.yml").read_text()
helper = (ROOT / ".github/workflows/helper-verify.yml").read_text()
evidence = (ROOT / ".github/workflows/release-evidence.yml").read_text()
ci = (ROOT / ".github/workflows/ci.yml").read_text()

for fragment in ('if [ "$GITHUB_EVENT_NAME" = workflow_dispatch ]; then',
                 'test "$GITHUB_RUN_ATTEMPT" = 1',
                 'python3 scripts/release_state.py promote --platform "$PLATFORM" --output dist',
                 'python3 scripts/release_state.py verify-dist --output dist',
                 'uses: ./.github/workflows/helper-verify.yml'):
    assert fragment in release, f"missing two-phase release requirement: {fragment}"
build = release.index("cargo build --locked --release --bin rhost")
assert release.index('if [ "$GITHUB_EVENT_NAME" = workflow_dispatch ]; then') < build < release.index("          else\n", build)
assert "uses: ./.github/workflows/helper-verify.yml" in ci
for fragment in ("workflow_call:", "workflow_dispatch:", "source_commit:", "manifest_sha256:", "asset_ids:",
                 "runner: ubuntu-24.04\n            target: linux-x86_64",
                 "runner: ubuntu-24.04-arm\n            target: linux-aarch64",
                 "cargo test --locked --test acceptance",
                 "cargo test --locked --features helper-tests --test helper_protocol --test helper_runs --test run_cli",
                 "sh scripts/test-helper-ssh.sh", "name: helper-${{ matrix.target }}"):
    assert fragment in helper, f"missing exact helper verification requirement: {fragment}"
for fragment in ("source_commit:", "candidate_run_id:", "client_sha256:", "helper_sha256:", "live_suites_passed:",
                 "release_state.py emit-evidence", "name: manual-live", "contents: read", "actions: read"):
    assert fragment in evidence, f"missing operator evidence requirement: {fragment}"
assert "contents: write" not in evidence + helper
for path in (ROOT / ".github/workflows").glob("*.yml"):
    text = path.read_text()
    assert not re.search(r"(?:sh|python\S*)\s+scripts/build-helper", text), f"CI must not compile helpers: {path.name}"
    for line in text.splitlines():
        if re.search(r"^\s*(?:-\s+)?uses:", line):
            assert re.search(r"@[0-9a-f]{40}", line) or line.strip() == "uses: ./.github/workflows/helper-verify.yml", f"unbound action: {line}"
    if path.name in {"ci.yml", "release.yml"}:
        assert "RHOST_HELPER_BIN:" in text and "RHOST_NO_SOURCE_BUILD: '1'" in text
        assert "helper_artifact.py fetch" in text
print("check-release-v5: source-locked helper verification and exact candidate promotion")
