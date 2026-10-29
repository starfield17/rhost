#!/usr/bin/env python3
"""Validate or fetch a source-locked, locally built helper. Never compile one."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parent.parent
SHA = re.compile(r"[0-9a-f]{64}")


def lock():
    value = json.loads((ROOT / "helper/artifacts.json").read_text())
    source = subprocess.check_output(["sh", "scripts/helper-source-digest.sh"], cwd=ROOT, text=True).strip()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    assert set(value) == {"format", "source_digest", "release", "toolchain", "artifacts"}
    assert value["format"] == 1 and value["source_digest"] == source
    assert value["release"] == f"v{version}"
    expected = {"linux-x86_64": "runtime", "linux-aarch64": "runtime", "darwin-aarch64": "fixture"}
    assert len(value["artifacts"]) == len(expected)
    for artifact in value["artifacts"]:
        assert set(artifact) == {"target", "asset", "sha256", "size", "role", "build_config"}
        assert expected.pop(artifact["target"]) == artifact["role"]
        assert artifact["asset"] == f"rhost-helper_{version}_{artifact['target']}"
        assert SHA.fullmatch(artifact["sha256"]) and SHA.fullmatch(artifact["build_config"])
        assert type(artifact["size"]) is int and 1 <= artifact["size"] <= 8388608
        config = value["toolchain"]["configurations"][artifact["target"]]
        encoded = json.dumps(config, sort_keys=True, separators=(",", ":")).encode()
        assert hashlib.sha256(encoded).hexdigest() == artifact["build_config"]
    assert not expected
    return value


def verify(artifact, path):
    assert path.is_file() and not path.is_symlink(), "candidate must be a regular file"
    assert path.stat().st_size == artifact["size"], "candidate size differs from lock"
    with path.open("rb") as stream:
        assert hashlib.file_digest(stream, "sha256").hexdigest() == artifact["sha256"], "candidate SHA differs from lock"
    path.chmod(0o700)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("operation", choices=["check", "verify", "fetch"])
    p.add_argument("--target", choices=["linux-x86_64", "linux-aarch64", "darwin-aarch64"])
    p.add_argument("--path", type=Path)
    p.add_argument("--production", action="store_true")
    args = p.parse_args()
    value = lock()
    source_commit = os.environ.get("HELPER_SOURCE_COMMIT")
    manifest_sha = os.environ.get("HELPER_MANIFEST_SHA256")
    if source_commit:
        assert re.fullmatch(r"[0-9a-f]{40}", source_commit)
        assert subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip() == source_commit
    if manifest_sha:
        assert SHA.fullmatch(manifest_sha)
        assert hashlib.sha256((ROOT / "helper/artifacts.json").read_bytes()).hexdigest() == manifest_sha
    if args.operation == "check":
        print("helper lock: exact source and three native candidates")
        return
    assert args.target and args.path, "set --target and --path"
    artifact = next(a for a in value["artifacts"] if a["target"] == args.target)
    if args.operation == "fetch":
        args.path.parent.mkdir(parents=True, exist_ok=True)
        release = value["release"] if args.production else f"helper-candidates-{value['source_digest']}"
        metadata = json.loads(subprocess.check_output(["gh", "api", f"repos/starfield17/rhost/releases/tags/{release}"]))
        candidates = [a for a in metadata["assets"] if a["name"] == artifact["asset"]]
        assert len(candidates) == 1 and candidates[0]["size"] == artifact["size"]
        selected = candidates[0]
        supplied_ids = os.environ.get("HELPER_ASSET_IDS")
        if supplied_ids:
            assert json.loads(supplied_ids)[args.target] == selected["id"]
        url = f"https://github.com/starfield17/rhost/releases/download/{release}/{artifact['asset']}"
        subprocess.run(["curl", "--fail", "--location", "--silent", "--show-error", "--proto", "=https",
                        "--proto-redir", "=https", "--max-time", "120", "--max-filesize", str(artifact["size"]),
                        "--output", args.path, url], check=True)
    verify(artifact, args.path)
    if args.operation == "fetch":
        proof = {"format": 1, "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                 "source_digest": value["source_digest"], "target": args.target, "sha256": artifact["sha256"],
                 "asset_id": selected["id"], "manifest_sha256": hashlib.sha256((ROOT / "helper/artifacts.json").read_bytes()).hexdigest(),
                 "run_id": os.environ.get("GITHUB_RUN_ID"), "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT")}
        (args.path.parent / f"helper-proof-{args.target}.json").write_text(json.dumps(proof, indent=2) + "\n")
    print(f"verified {artifact['target']} {artifact['sha256']}")


if __name__ == "__main__":
    main()
