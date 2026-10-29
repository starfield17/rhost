"""Repository-scoped, fail-closed access to immutable Actions artifacts."""
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import zipfile

REPO = "starfield17/rhost"
PLATFORMS = ("darwin_amd64", "darwin_arm64", "linux_amd64", "linux_arm64")
SHA = re.compile(r"[0-9a-f]{64}")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def identifier(value):
    require(str(value).isdigit() and int(value) > 0, "invalid Actions identifier")
    return int(value)


def api(path, binary=False):
    require(os.environ.get("GITHUB_REPOSITORY", REPO) == REPO, "wrong repository")
    result = subprocess.check_output(["gh", "api", f"repos/{REPO}/{path}"])
    return result if binary else json.loads(result)


def run(run_id, commit, workflow):
    require(re.fullmatch(r"[0-9a-f]{40}", commit), "source commit must be full SHA")
    value = api(f"actions/runs/{identifier(run_id)}")
    require(value["head_sha"] == commit and value["path"] == f".github/workflows/{workflow}", "run source/workflow mismatch")
    require(value["event"] == "workflow_dispatch" and value["run_attempt"] == 1, "only first-attempt manual candidates are promotable")
    require(value["status"] == "completed" and value["conclusion"] == "success", "candidate run must succeed")
    require(value["repository"]["full_name"] == REPO and value["head_repository"]["full_name"] == REPO, "foreign run")
    return value


def artifacts(run_id):
    result = api(f"actions/runs/{identifier(run_id)}/artifacts?per_page=100")
    require(result["total_count"] == len(result["artifacts"]) <= 100, "incomplete artifact inventory")
    return result["artifacts"]


def archive(artifact):
    require(not artifact["expired"], "artifact expired; new candidate and live evidence required")
    digest = artifact.get("digest", "")
    require(re.fullmatch(r"sha256:[0-9a-f]{64}", digest), "missing archive digest")
    require(0 < artifact["size_in_bytes"] <= 128 * 1024 * 1024, "artifact archive exceeds limit")
    raw = api(f"actions/artifacts/{identifier(artifact['id'])}/zip", binary=True)
    require(len(raw) <= 128 * 1024 * 1024 and hashlib.sha256(raw).hexdigest() == digest[7:], "archive digest mismatch")
    result = {}
    with zipfile.ZipFile(io.BytesIO(raw)) as zipped:
        require(len(zipped.infolist()) <= 4, "unexpected archive entries")
        for entry in zipped.infolist():
            require(entry.filename == Path(entry.filename).name and entry.filename not in result, "unsafe archive name")
            require(not entry.is_dir() and entry.file_size <= 64 * 1024 * 1024, "unsafe archive entry")
            require((entry.external_attr >> 16) & 0o170000 != 0o120000, "archive symlink refused")
            result[entry.filename] = zipped.read(entry)
    return result


def candidate(run_id, commit, version, platform, output=None):
    run(run_id, commit, "release.yml")
    entries = artifacts(run_id)
    require(len(entries) == 6, "candidate requires four clients and two verified helpers")
    expected = {f"rhost-{p}" for p in PLATFORMS} | {"helper-linux-x86_64", "helper-linux-aarch64"}
    require({a["name"] for a in entries} == expected, "incomplete candidate platforms")
    selected = next(a for a in entries if a["name"] == f"rhost-{platform}")
    require(selected["workflow_run"]["id"] == int(run_id), "artifact belongs to another run")
    files = archive(selected)
    asset = f"rhost_{version}_{platform}"
    meta = f"candidate-{platform}.json"
    require(set(files) == {asset, f"{asset}.sha256", meta}, "unexpected client archive inventory")
    value = json.loads(files[meta])
    require(set(value) == {"format", "commit", "version", "platform", "sha256"}, "invalid candidate record")
    sha = hashlib.sha256(files[asset]).hexdigest()
    require(value == {"format": 1, "commit": commit, "version": version, "platform": platform, "sha256": sha}, "client record mismatch")
    require(files[f"{asset}.sha256"].decode() == f"{sha}  {asset}\n", "client checksum mismatch")
    if output:
        output.mkdir(parents=True, exist_ok=True)
        for name, data in files.items():
            (output / name).write_bytes(data)
        (output / asset).chmod(0o755)
    return {"artifact_id": selected["id"], "archive_sha256": selected["digest"][7:],
            "asset": asset, "sha256": sha, "size": len(files[asset])}


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--platform", choices=PLATFORMS, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    print(json.dumps(candidate(args.run, args.commit, args.version, args.platform, args.output)))
