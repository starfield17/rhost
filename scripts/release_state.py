#!/usr/bin/env python3
"""Bind production tag promotion to exact candidates and manual live evidence."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tomllib
from release_api import PLATFORMS, archive, artifacts, candidate, identifier, require, run
from helper_artifact import lock as helper_lock

ROOT = Path(__file__).resolve().parent.parent


def context():
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    return commit, version


def clients(run_id, commit, version):
    return {p: candidate(run_id, commit, version, p) for p in PLATFORMS}


def evidence(run_id, commit, version, candidate_run):
    observed = run(run_id, commit, "release-evidence.yml")
    entries = artifacts(run_id)
    require(len(entries) == 1 and entries[0]["name"] == "manual-live", "one manual evidence artifact required")
    files = archive(entries[0])
    require(set(files) == {"manual-live.json"}, "invalid evidence archive")
    value = json.loads(files["manual-live.json"])
    require(set(value) == {"format", "kind", "observed_by_ci", "commit", "version", "candidate_run_id", "run_attempt",
                           "evidence_run_id", "operator", "recorded_at", "clients", "manual_client_platform", "helper_sha256", "helper_manifest_sha256", "suites"}, "invalid live evidence fields")
    require(value["format"] == 1 and value["kind"] == "operator_attestation" and value["observed_by_ci"] is False, "not an operator attestation")
    require(value["commit"] == commit and value["version"] == version and value["candidate_run_id"] == int(candidate_run), "evidence source mismatch")
    require(value["run_attempt"] == 1 and value["evidence_run_id"] == int(run_id), "evidence attempt mismatch")
    require(value["operator"] == observed["triggering_actor"]["login"], "evidence operator mismatch")
    datetime.datetime.strptime(value["recorded_at"], "%Y-%m-%dT%H:%M:%SZ")
    require(value["suites"] == {"test-live-all": "passed", "test-live-helper": "passed"}, "required manual suites did not pass")
    require(value["manual_client_platform"] in PLATFORMS, "unknown manual client")
    lock = helper_lock()
    require(value["helper_manifest_sha256"] == hashlib.sha256((ROOT / "helper/artifacts.json").read_bytes()).hexdigest(), "live helper manifest differs")
    require(value["helper_sha256"] in {a["sha256"] for a in lock["artifacts"] if a["role"] == "runtime"}, "live helper differs")
    require(value["clients"] == clients(candidate_run, commit, version), "live candidate identity differs")
    return value, entries[0]["id"], files["manual-live.json"]


def emit(args):
    commit, version = context()
    require(os.environ["SOURCE_COMMIT"] == commit == os.environ["GITHUB_SHA"], "dispatch ref must be fixed at the source commit")
    require(os.environ["GITHUB_RUN_ATTEMPT"] == "1", "reruns need new evidence")
    selected = os.environ["CLIENT_PLATFORM"]
    all_clients = clients(args.candidate_run, commit, version)
    require(selected in all_clients and all_clients[selected]["sha256"] == os.environ["CLIENT_SHA256"], "manually tested client mismatch")
    helper = os.environ["HELPER_SHA256"]
    require(helper in {a["sha256"] for a in helper_lock()["artifacts"] if a["role"] == "runtime"}, "manually tested helper mismatch")
    require(os.environ["LIVE_SUITES_PASSED"] == "test-live-all,test-live-helper", "explicit manual suite attestation required")
    value = {"format": 1, "kind": "operator_attestation", "observed_by_ci": False, "commit": commit, "version": version,
             "candidate_run_id": identifier(args.candidate_run), "run_attempt": 1, "evidence_run_id": identifier(os.environ["GITHUB_RUN_ID"]),
             "operator": os.environ["GITHUB_ACTOR"], "recorded_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
             "clients": all_clients, "manual_client_platform": selected, "helper_sha256": helper,
             "helper_manifest_sha256": hashlib.sha256((ROOT / "helper/artifacts.json").read_bytes()).hexdigest(),
             "suites": {"test-live-all": "passed", "test-live-helper": "passed"}}
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "manual-live.json").write_text(json.dumps(value, indent=2) + "\n")


def annotation():
    commit, version = context()
    tag = os.environ["GITHUB_REF_NAME"]
    require(tag == f"v{version}", "tag does not match Cargo version")
    require(subprocess.check_output(["git", "cat-file", "-t", tag], text=True).strip() == "tag", "annotated production tag required")
    text = subprocess.check_output(["git", "cat-file", "-p", tag], text=True)
    value = json.loads(text.split("\n\n", 1)[1])
    require(set(value) == {"format", "commit", "candidate_run_id", "candidate_artifacts", "evidence_run_id", "evidence_artifact_id"}, "invalid production annotation")
    require(value["format"] == 1 and value["commit"] == commit, "production annotation source mismatch")
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["emit-evidence", "tag-message", "promote", "verify-dist"])
    parser.add_argument("--candidate-run")
    parser.add_argument("--evidence-run")
    parser.add_argument("--platform", choices=PLATFORMS)
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    if args.operation == "emit-evidence":
        emit(args)
        return
    commit, version = context()
    binding = annotation() if args.operation in {"promote", "verify-dist"} else {"candidate_run_id": identifier(args.candidate_run), "evidence_run_id": identifier(args.evidence_run)}
    value, artifact_id, raw = evidence(binding["evidence_run_id"], commit, version, binding["candidate_run_id"])
    ids = {p: v["artifact_id"] for p, v in value["clients"].items()}
    result = {"format": 1, "commit": commit, "candidate_run_id": binding["candidate_run_id"],
              "candidate_artifacts": ids, "evidence_run_id": binding["evidence_run_id"], "evidence_artifact_id": artifact_id}
    if args.operation == "tag-message":
        print(json.dumps(result, sort_keys=True))
        return
    require(binding == result, "annotated immutable artifact IDs differ")
    if args.operation == "verify-dist":
        from helper_artifact import verify
        expected = {"manual-live.json", "helper-artifacts.json"}
        for platform, client in value["clients"].items():
            asset = client["asset"]
            expected.update({asset, f"{asset}.sha256", f"candidate-{platform}.json"})
            require(hashlib.sha256((args.output / asset).read_bytes()).hexdigest() == client["sha256"], "publication client differs from live-bound candidate")
            require((args.output / f"{asset}.sha256").read_text() == f"{client['sha256']}  {asset}\n", "publication checksum differs")
        for artifact in helper_lock()["artifacts"]:
            if artifact["role"] != "runtime":
                continue
            verify(artifact, args.output / artifact["asset"])
            proof_name = f"helper-proof-{artifact['target']}.json"
            proof = json.loads((args.output / proof_name).read_text())
            require(proof["source_commit"] == commit and proof["sha256"] == artifact["sha256"] and proof["target"] == artifact["target"], "publication helper proof differs")
            require(proof["run_id"] == os.environ["GITHUB_RUN_ID"] and proof["run_attempt"] == "1", "helper proof must come from this first-attempt tag verification")
            expected.update({artifact["asset"], proof_name})
        require((args.output / "helper-artifacts.json").read_bytes() == (ROOT / "helper/artifacts.json").read_bytes(), "publication helper lock differs")
        require((args.output / "manual-live.json").read_bytes() == raw, "publication evidence differs")
        require({p.name for p in args.output.iterdir()} == expected, "unexpected publication inventory")
        print("verified exact live-bound clients, freshly tested helpers and publication inventory")
        return
    if args.platform:
        candidate(binding["candidate_run_id"], commit, version, args.platform, args.output)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "manual-live.json").write_bytes(raw)


if __name__ == "__main__":
    main()
