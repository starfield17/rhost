//! Offline acceptance of annotated-tag, operator-evidence and publication gates.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(1);
const PLATFORMS: [&str; 4] = ["darwin_amd64", "darwin_arm64", "linux_amd64", "linux_arm64"];
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
struct Fixture {
    root: PathBuf,
    commit: String,
    evidence: Value,
    binding: Value,
}
impl Fixture {
    fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> Result<(), String> {
        let path = self.root.join(name);
        std::fs::create_dir_all(path.parent().ok_or("parent")?).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }
    fn git(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!("git {args:?}: {out:?}"));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().into())
    }
    fn reply(&self, endpoint: &str, bytes: impl AsRef<[u8]>) -> Result<(), String> {
        self.write(
            &format!(
                "replies/repos+starfield17+rhost+{}",
                endpoint.replace('/', "+")
            ),
            bytes,
        )
    }
    fn packed(
        &self,
        id: u64,
        name: &str,
        files: &[(&str, Vec<u8>)],
        run: u64,
    ) -> Result<Value, String> {
        let directory = self.root.join(format!("archive-{id}"));
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        for (path, bytes) in files {
            std::fs::write(directory.join(path), bytes).map_err(|e| e.to_string())?;
        }
        let path = self.root.join(format!("archive-{id}.zip"));
        let _ = std::fs::remove_file(&path);
        let out = Command::new("zip")
            .current_dir(&directory)
            .arg("-q")
            .arg(&path)
            .args(files.iter().map(|(name, _)| name))
            .output()
            .map_err(|e| e.to_string())?;
        assert!(out.status.success(), "{out:?}");
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        self.reply(&format!("actions/artifacts/{id}/zip"), &bytes)?;
        Ok(
            json!({"id":id,"name":name,"expired":false,"size_in_bytes":bytes.len(),"digest":format!("sha256:{}",hash(&bytes)),"workflow_run":{"id":run}}),
        )
    }
    fn save_evidence(&self) -> Result<(), String> {
        let raw = serde_json::to_vec(&self.evidence).map_err(|e| e.to_string())?;
        self.write("dist/manual-live.json", &raw)?;
        let artifact = self.packed(30, "manual-live", &[("manual-live.json", raw)], 6)?;
        self.reply(
            "actions/runs/6/artifacts?per_page=100",
            json!({"total_count":1,"artifacts":[artifact]}).to_string(),
        )
    }
    fn tag(&self) -> Result<(), String> {
        let _ = self.git(&["tag", "-d", "v5.0.0"]);
        self.write("annotation", self.binding.to_string())?;
        self.git(&["tag", "-a", "v5.0.0", "-F", "annotation"])?;
        Ok(())
    }
    fn new() -> Result<Self, String> {
        let mut f = Self {
            root: std::env::temp_dir().join(format!(
                "rhost-release-state-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            )),
            commit: String::new(),
            evidence: Value::Null,
            binding: Value::Null,
        };
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
        for file in [
            "release_state.py",
            "release_api.py",
            "helper_artifact.py",
            "helper-source-digest.sh",
        ] {
            f.write(
                &format!("scripts/{file}"),
                std::fs::read(repository.join("scripts").join(file)).map_err(|e| e.to_string())?,
            )?;
        }
        f.write(
            "Cargo.toml",
            "[package]\nname='release-fixture'\nversion='5.0.0'\n",
        )?;
        f.write("helper/fixture.c", "/* source fixture */\n")?;
        let source = hash(b"helper/fixture.c\0/* source fixture */\n\0");
        let config = json!({});
        let config_hash = hash(b"{}");
        let mut helpers = Vec::new();
        let mut configs = serde_json::Map::new();
        for target in ["linux-x86_64", "linux-aarch64", "darwin-aarch64"] {
            let asset = format!("rhost-helper_5.0.0_{target}");
            let bytes = target.as_bytes();
            configs.insert(target.into(), config.clone());
            helpers.push(json!({"target":target,"asset":asset,"sha256":hash(bytes),"size":bytes.len(),"role":if target.starts_with("linux-"){"runtime"}else{"fixture"},"build_config":config_hash}));
            if target.starts_with("linux-") {
                f.write(&format!("dist/{asset}"), bytes)?;
            }
        }
        let lock = json!({"format":1,"source_digest":source,"release":"v5.0.0","toolchain":{"configurations":configs},"artifacts":helpers});
        let raw_lock = lock.to_string();
        f.write("helper/artifacts.json", &raw_lock)?;
        f.write("dist/helper-artifacts.json", &raw_lock)?;
        f.git(&["init", "-q"])?;
        f.git(&["config", "user.name", "Test"])?;
        f.git(&["config", "user.email", "test@example.invalid"])?;
        f.git(&[
            "add",
            "Cargo.toml",
            "helper/fixture.c",
            "helper/artifacts.json",
        ])?;
        f.git(&["commit", "-qm", "source fixture"])?;
        f.commit = f.git(&["rev-parse", "HEAD"])?;
        f.write("bin/gh", b"#!/bin/sh\nendpoint=$(printf '%s' \"$2\" | tr / +)\nexec /bin/cat \"$RHOST_GH_FIXTURES/$endpoint\"\n")?;
        std::fs::set_permissions(
            f.root.join("bin/gh"),
            std::fs::Permissions::from_mode(0o700),
        )
        .map_err(|e| e.to_string())?;
        for (id, workflow) in [(5, "release.yml"), (6, "release-evidence.yml")] {
            f.reply(&format!("actions/runs/{id}"), json!({"head_sha":f.commit,"path":format!(".github/workflows/{workflow}"),"event":"workflow_dispatch","run_attempt":1,"status":"completed","conclusion":"success","repository":{"full_name":"starfield17/rhost"},"head_repository":{"full_name":"starfield17/rhost"},"triggering_actor":{"login":"release-operator"}}).to_string())?;
        }
        let mut clients = serde_json::Map::new();
        let mut ids = serde_json::Map::new();
        let mut archives = Vec::new();
        for (index, platform) in PLATFORMS.into_iter().enumerate() {
            let id = 10 + index as u64;
            let asset = format!("rhost_5.0.0_{platform}");
            let checksum = format!("{asset}.sha256");
            let metadata = format!("candidate-{platform}.json");
            let bytes = format!("client {platform}").into_bytes();
            let sha = hash(&bytes);
            let files = [(&*asset, bytes.clone()), (&*checksum, format!("{sha}  {asset}\n").into_bytes()), (&*metadata, json!({"format":1,"commit":f.commit,"version":"5.0.0","platform":platform,"sha256":sha}).to_string().into_bytes())];
            for (name, bytes) in &files {
                f.write(&format!("dist/{name}"), bytes)?;
            }
            let archive = f.packed(id, &format!("rhost-{platform}"), &files, 5)?;
            clients.insert(platform.into(), json!({"artifact_id":id,"archive_sha256":archive["digest"].as_str().ok_or("digest")?[7..],"asset":asset,"sha256":sha,"size":bytes.len()}));
            ids.insert(platform.into(), id.into());
            archives.push(archive);
        }
        for (id, target) in [(20, "linux-x86_64"), (21, "linux-aarch64")] {
            archives.push(json!({"id":id,"name":format!("helper-{target}")}));
            f.write(&format!("dist/helper-proof-{target}.json"), json!({"source_commit":f.commit,"target":target,"sha256":hash(target.as_bytes()),"run_id":"7","run_attempt":"1"}).to_string())?;
        }
        f.reply(
            "actions/runs/5/artifacts?per_page=100",
            json!({"total_count":6,"artifacts":archives}).to_string(),
        )?;
        f.evidence = json!({"format":1,"kind":"operator_attestation","observed_by_ci":false,"commit":f.commit,"version":"5.0.0","candidate_run_id":5,"run_attempt":1,"evidence_run_id":6,"operator":"release-operator","recorded_at":"2026-01-01T00:00:00Z","clients":clients,"manual_client_platform":"darwin_arm64","helper_sha256":hash(b"linux-aarch64"),"helper_manifest_sha256":hash(raw_lock.as_bytes()),"suites":{"test-live-all":"passed","test-live-helper":"passed"}});
        f.binding = json!({"format":1,"commit":f.commit,"candidate_run_id":5,"candidate_artifacts":ids,"evidence_run_id":6,"evidence_artifact_id":30});
        f.save_evidence()?;
        f.tag()?;
        Ok(f)
    }
    fn invoke(&self, operation: &str) -> Result<Output, String> {
        Command::new("python3")
            .current_dir(&self.root)
            .args(["scripts/release_state.py", operation, "--output", "dist"])
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("RHOST_GH_FIXTURES", self.root.join("replies"))
            .env("GITHUB_REPOSITORY", "starfield17/rhost")
            .env("GITHUB_REF_NAME", "v5.0.0")
            .env("GITHUB_RUN_ID", "7")
            .output()
            .map_err(|e| e.to_string())
    }
    fn refused(&self, operation: &str, reason: &str) -> Result<(), String> {
        let out = self.invoke(operation)?;
        assert!(!out.status.success(), "accepted {reason}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(reason),
            "{out:?}"
        );
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn production_tag_requires_annotation_and_exact_evidence_artifact_identity() -> Result<(), String> {
    let mut f = Fixture::new()?;
    let good = f.invoke("promote")?;
    assert!(good.status.success(), "{good:?}");
    f.git(&["tag", "-d", "v5.0.0"])?;
    f.git(&["tag", "v5.0.0"])?;
    f.refused("promote", "annotated production tag required")?;
    f.binding["commit"] = "b".repeat(40).into();
    f.tag()?;
    f.refused("promote", "production annotation source mismatch")?;
    f.binding["commit"] = f.commit.clone().into();
    f.binding["evidence_artifact_id"] = 31.into();
    f.tag()?;
    f.refused("promote", "annotated immutable artifact IDs differ")
}

#[test]
fn operator_evidence_cannot_substitute_commit_client_or_helper() -> Result<(), String> {
    let mut f = Fixture::new()?;
    let original = f.evidence.clone();
    for (pointer, replacement, reason) in [
        (
            "/operator",
            json!("another-operator"),
            "evidence operator mismatch",
        ),
        ("/commit", json!("b".repeat(40)), "evidence source mismatch"),
        (
            "/helper_sha256",
            json!("0".repeat(64)),
            "live helper differs",
        ),
        (
            "/clients/darwin_arm64/sha256",
            json!("0".repeat(64)),
            "live candidate identity differs",
        ),
        (
            "/manual_client_platform",
            json!("linux_arm64"),
            "manual witness requires a native macOS client",
        ),
    ] {
        f.evidence = original.clone();
        *f.evidence.pointer_mut(pointer).ok_or("pointer")? = replacement;
        f.save_evidence()?;
        f.refused("promote", reason)?;
    }
    Ok(())
}

#[test]
fn publication_inventory_and_fresh_helper_proofs_are_required() -> Result<(), String> {
    let f = Fixture::new()?;
    let good = f.invoke("verify-dist")?;
    assert!(good.status.success(), "{good:?}");
    f.write("dist/unexpected", b"extra")?;
    f.refused("verify-dist", "unexpected publication inventory")?;
    std::fs::remove_file(f.root.join("dist/unexpected")).map_err(|e| e.to_string())?;
    let proof_name = "dist/helper-proof-linux-aarch64.json";
    let original = std::fs::read(f.root.join(proof_name)).map_err(|e| e.to_string())?;
    for (field, value) in [("run_id", "8"), ("run_attempt", "2")] {
        let mut proof: Value = serde_json::from_slice(&original).map_err(|e| e.to_string())?;
        proof[field] = value.into();
        f.write(proof_name, proof.to_string())?;
        f.refused(
            "verify-dist",
            "helper proof must come from this first-attempt tag verification",
        )?;
    }
    f.write(proof_name, original)?;
    f.write("dist/rhost_5.0.0_darwin_arm64", b"substitution")?;
    f.refused("verify-dist", "publication client differs")?;
    std::fs::remove_file(f.root.join("dist/rhost_5.0.0_darwin_arm64"))
        .map_err(|e| e.to_string())?;
    f.refused("verify-dist", "No such file")
}
