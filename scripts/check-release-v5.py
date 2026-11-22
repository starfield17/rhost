#!/usr/bin/env python3
"""Enforce the native helper release policy shared by v5 and v6."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parent.parent


def workflow(name):
    text = (ROOT / ".github/workflows" / name).read_text()
    return "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))


release = workflow("release.yml")
helper = workflow("helper-verify.yml")
ci = workflow("ci.yml")
publish = release.split("\n  publish:\n", 1)[1]

for fragment in ("workflow_call:", "workflow_dispatch:",
                 "runner: ubuntu-24.04\n            target: linux-x86_64",
                 "runner: ubuntu-24.04-arm\n            target: linux-aarch64",
                 "runner: macos-15\n            target: darwin-aarch64",
                 'asset="rhost-helper_${version}_${TARGET}"',
                 'CC=musl-gcc RHOST_HELPER_STATIC=1 sh scripts/build-helper.sh "$helper"',
                 'CC=clang sh scripts/build-helper.sh "$helper"',
                 'shasum -a 256 "$asset" > "$asset.sha256"',
                 "cargo test --locked --test acceptance",
                 "cargo test --locked --features helper-tests --test helper_protocol --test helper_runs --test run_cli",
                 "sh scripts/test-helper-ssh.sh",
                 'shasum -a 256 -c "${RHOST_HELPER_BIN##*/}.sha256"',
                 "cargo test --locked --features helper-tests,helper-fault-tests --test helper_faults",
                 "cargo test --locked --features helper-tests --test helper_protocol --test helper_runs --test run_cli --test acceptance",
                 "ASAN_OPTIONS: detect_leaks=1", "UBSAN_OPTIONS: halt_on_error=1",
                 "name: helper-${{ matrix.target }}", "RHOST_NO_SOURCE_BUILD: '1'", "RHOST_NO_DOWNLOAD: '1'"):
    assert fragment in helper, f"missing native helper verification requirement: {fragment}"
assert "contents: write" not in helper
ssh_step = helper.split("- name: Execute real SSH acceptance against disposable sshd\n", 1)[1]
ssh_step = ssh_step.split("\n      - ", 1)[0]
assert "if: runner.os == 'Linux'" in ssh_step
assert "run: sh scripts/test-helper-ssh.sh" in ssh_step
assert helper.index('shasum -a 256 "$asset"') < helper.index("cargo test --locked --test acceptance")
assert helper.index("sh scripts/test-helper-ssh.sh") < helper.index('shasum -a 256 -c')
assert helper.index('shasum -a 256 -c') < helper.index("actions/upload-artifact@")
for workflow in (ci, release):
    assert "uses: ./.github/workflows/helper-verify.yml" in workflow
    assert 'run: make check' in workflow
    assert './scripts/check-integrity.sh "$base"' in workflow or './scripts/check-integrity.sh "$BASE"' in workflow
assert "pattern: rhost-*" in publish and "pattern: helper-linux-*" in publish
assert publish.index('sh scripts/verify-release-assets.sh "$version" dist') < publish.index('gh release create')
for path in (ROOT / ".github/workflows").glob("*.yml"):
    for line in path.read_text().splitlines():
        if re.search(r"^\s*(?:-\s+)?uses:", line):
            assert re.search(r"@[0-9a-f]{40}$", line) or line.strip() == "uses: ./.github/workflows/helper-verify.yml", f"unbound action: {line}"
assert '#[path = "support/helper_v6.rs"]' in (ROOT / "tests/helper_runs.rs").read_text()
print("check-release-v5: three native helpers, exact-byte acceptance and SHA-256 publication checks")
