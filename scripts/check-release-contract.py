#!/usr/bin/env python3
"""Enforce the native release workflow and installer contract."""
from pathlib import Path
import json
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parent.parent
ISSUES: list[str] = []


def report(message: str) -> None:
    ISSUES.append(message)
    print(message)


def count_line(text: str, fragment: str) -> int:
    return sum(fragment in line for line in text.splitlines())


def require_once(text: str, label: str, fragment: str) -> None:
    count = count_line(text, fragment)
    if count != 1:
        report(f"expected exactly one '{label}', found {count}")


def refuse(text: str, label: str, pattern: str) -> None:
    found = [f"{n}:{line}" for n, line in enumerate(text.splitlines(), 1) if re.search(pattern, line)]
    if found:
        report(f"must not appear ({label}):")
        print("\n".join(found))


def job_text(workflow: str, wanted: str) -> str:
    lines = []
    inside = False
    for line in workflow.splitlines():
        if re.match(r"^  [a-zA-Z_-]+:", line):
            inside = line == f"  {wanted}:"
        if inside:
            lines.append(line)
    return "\n".join(lines)


def require_job_once(workflow: str, job: str, fragment: str) -> None:
    count = count_line(job_text(workflow, job), fragment)
    if count != 1:
        report(f"expected exactly one '{fragment}' in {job}, found {count}")


def main() -> int:
    if sys.argv[1:] not in ([], ["-v"], ["--verbose"]):
        print(f"usage: {sys.argv[0]} [-v]", file=sys.stderr)
        return 2
    verbose = bool(sys.argv[1:])
    needed = [".github/workflows/release.yml", "Cargo.toml", "scripts/install.sh",
              "plugin.json", ".codex-plugin/plugin.json", "README.md", "AGENTS.md", "docs/MAINTENANCE.md"]
    for name in needed:
        if not (ROOT / name).is_file():
            print(f"check-release-contract: {name} is missing", file=sys.stderr)
            return 2
    workflow = (ROOT / needed[0]).read_text()
    installer = (ROOT / "scripts/install.sh").read_text()
    expected_pairs = [("macos-15-intel", "darwin_amd64"), ("macos-15", "darwin_arm64"),
                      ("ubuntu-24.04", "linux_amd64"), ("ubuntu-24.04-arm", "linux_arm64")]
    pairs = []
    pending = None
    for line in workflow.splitlines():
        runner = re.match(r"^\s*-\s*runner:\s*(\S+)\s*$", line)
        platform = re.match(r"^\s*platform:\s*(\S+)\s*$", line)
        if runner:
            pending = runner.group(1)
        elif platform and pending:
            pairs.append((pending, platform.group(1)))
            pending = None
    if pairs != expected_pairs:
        report("the matrix is not the four shipping platform/runner pairs:")
        print(f"expected: {expected_pairs}\nfound: {pairs}")
    require_once(workflow, "locked release build", "cargo build --locked --release --bin rhost")
    require_once(workflow, "source verification job", "  verify:")
    require_job_once(workflow, "verify", "        os: [ubuntu-24.04, macos-15]")
    require_job_once(workflow, "verify", "    runs-on: ${{ matrix.os }}")
    require_job_once(workflow, "verify", "      - run: make check")
    require_once(workflow, "complete source gate", "      - run: make check")
    require_job_once(workflow, "native", "    needs: verify")
    require_job_once(workflow, "publish", "    needs: native")
    require_job_once(workflow, "verify", 'rustup component add --toolchain "$rust_version" rustfmt clippy')
    refuse(workflow, "a redundant smoke suite", r"make test-smoke")
    refuse(workflow, "a live suite the release workflow cannot actually reach", r"test-live|RHOST_TEST_HOST")
    for claim in ("`test-live-all` is the serial, Rust-native real-SSH release gate",
                  "Live suites are the required real-remote verification gate"):
        for name in ("README.md", "AGENTS.md"):
            if claim in (ROOT / name).read_text():
                report(f"{name} still claims a workflow-enforced live gate: {claim}")
    for name in ("README.md", "AGENTS.md", "docs/MAINTENANCE.md"):
        if not re.search(r"manual[*_ ]*pre-release", (ROOT / name).read_text(), re.I):
            report(f"{name} does not state that real-remote verification is manual pre-release")
    toolchain_read = 'rust_version=$(sed -n \'s/^rust-version = "\\([^"]*\\)"/\\1/p\' Cargo.toml)'
    python_action = "actions/setup-python@ece7cb06caefa5fff74198d8649806c4678c61a1"
    for job in ("verify", "native"):
        require_job_once(workflow, job, toolchain_read)
        require_job_once(workflow, job, 'rustup toolchain install "$rust_version" --profile minimal')
        require_job_once(workflow, job, 'rustup override set "$rust_version"')
        require_job_once(workflow, job, python_action)
        require_job_once(workflow, job, "          python-version: '3.11'")
    require_once(workflow, "self-check step", "./scripts/check-release-contract.sh")
    makefile = (ROOT / "Makefile").read_text()
    build_target = re.search(r"^build-release:\s*\n((?:[ \t].*\n)*)", makefile, re.M)
    build_text = build_target.group(1) if build_target else ""
    for fragment in ("cargo build --locked --release --bin rhost", "RHOST_BUILD_COMMIT", "RHOST_BUILD_DATE"):
        if fragment not in build_text:
            report(f"make build-release is missing: {fragment}")
    if "--debug" in build_text:
        report("make build-release must not build the debug profile")
    require_once(workflow, "version read from Cargo.toml", r'''version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml)''')
    refuse(workflow, "a second version authority", r"plugin\.json|(/|\s)VERSION([^_A-Z]|$)")
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    version = manifest["package"]["version"]
    for name in ("plugin.json", ".codex-plugin/plugin.json"):
        if json.loads((ROOT / name).read_text()).get("version") != version:
            report(f"{name} version must mirror Cargo.toml")
    for label, fragment in (
        ("artifact name", 'artifact="dist/rhost_${version}_${PLATFORM}"'),
        ("artifact executes version --json", '"$artifact" version --json'),
        ("artifact version matches the manifest", 'assert d["data"]["version"] == sys.argv[1]'),
        ("artifact commit matches the checkout", 'assert d["data"]["commit"] == sys.argv[1]'),
        ("artifact build date is a UTC timestamp", 're.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z"'),
        ("artifact executes --help", '"$artifact" --help'),
        ("artifact basename selected", 'asset_name=${artifact##*/}'),
        ("basename checksum written", 'shasum -a 256 "$asset_name" > "$asset_name.sha256"'),
        ("basename checksum verified", 'shasum -a 256 -c "$asset_name.sha256"'),
    ):
        require_once(workflow, label, fragment)
    if "PY_INSTALLER" in installer:
        for fragment in ('"Darwin": "darwin"', '"Linux": "linux"',
                         '"arm64": "arm64"', '"aarch64": "arm64"',
                         '"x86_64": "amd64"', '"amd64": "amd64"',
                         'f"rhost_{version}_{os_name}_{arch}"',
                         'f"https://github.com/{REPO}/releases/download/v{version}/{asset}"',
                         "hashlib.sha256", "os.replace"):
            if fragment not in installer:
                report(f"install.sh is missing release mapping or verification: {fragment}")
    else:
        for fragment in ('darwin) os=darwin', 'linux)  os=linux', 'arm64|aarch64) arch=arm64',
                         'x86_64|amd64)  arch=amd64', 'asset="rhost_${version}_${os}_${arch}"',
                         'releases/download/v${version}/${asset}'):
            if fragment not in installer:
                report(f"install.sh is missing release mapping: {fragment}")
        if not re.search(r"sha256sum|shasum", installer):
            report("install.sh does not verify SHA-256")
    for label, fragment in (
        ("version-tag trigger", '      - "v*"'),
        ("publication tag guard", "    if: startsWith(github.ref, 'refs/tags/v')"),
        ("publication waits for native artifacts", "    needs: native"),
        ("release write permission", "      contents: write"),
        ("all artifacts downloaded together", "          merge-multiple: true"),
        ("tag matches Cargo version", '          if [ "$GITHUB_REF_TYPE" = tag ]; then test "$GITHUB_REF_NAME" = "v${version}"; fi'),
        ("eight release files present", '          test "$(find dist -maxdepth 1 -type f -name \'rhost_*\' | wc -l | tr -d \' \')" = 8'),
        ("GitHub release publication", '          gh release create "v${version}" dist/* --verify-tag --title "rhost v${version}" --generate-notes'),
    ):
        require_once(workflow, label, fragment)
    publish = job_text(workflow, "publish")
    checkouts = re.findall(r"^\s*-\s+uses: actions/checkout@[0-9a-f]{40}$", publish, re.M)
    if len(checkouts) != 1:
        report("the publish job must check out the tagged repository exactly once")
    refuse(workflow, "a second publisher", r"action-gh-release|actions/create-release|cargo publish|git tag |git push|docker push|npm publish")
    refuse(workflow, "an unrelated write permission", r"id-token: write|packages: write")
    refuse(workflow, "a cross-compiler", r"\-\-target|qemu|cross build|\bzig\b")
    refuse(workflow, "a read-only reference tree", r"reference/")
    trigger_match = re.search(r"^on:\n((?:[ \t].*\n|[ \t]*\n)*)", workflow, re.M)
    triggers = [line for line in (trigger_match.group(1) if trigger_match else "").splitlines()
                if line.strip() and not line.lstrip().startswith("#")]
    if triggers != ["  workflow_dispatch:", "  push:", "    tags:", '      - "v*"']:
        report("the workflow triggers are not manual rehearsal plus version tags:")
        print("\n".join(triggers))
    unpinned = [f"{number}:{line}" for number, line in enumerate(workflow.splitlines(), 1)
                if re.search(r"^\s*(?:-\s+)?uses:", line) and not re.search(r"@[0-9a-f]{40}", line)]
    if unpinned:
        report("every action must be pinned to a 40-character commit:")
        print("\n".join(unpinned))
    if ISSUES:
        print("\nFound release-contract violations.\n\nThe release workflow and this check are described in docs/CONTRACT.md\n(RELEASE-001) and docs/MAINTENANCE.md.", file=sys.stderr)
        return 1
    if verbose:
        for runner, platform in pairs:
            print(f"{runner} {platform}")
    print(f"check-release-contract: OK (2 source platforms, {len(pairs)} native platform/runner pairs, tag-gated publication)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
