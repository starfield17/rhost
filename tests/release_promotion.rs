//! The release API boundary rejects mutable run attempts and substituted bytes.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(1);
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
struct Fixture {
    root: PathBuf,
    run: Value,
    artifacts: Value,
}
impl Fixture {
    fn new() -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!(
            "rhost-promotion-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("bin")).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(root.join("files")).map_err(|e| e.to_string())?;
        let gh = root.join("bin/gh");
        std::fs::write(
            &gh,
            b"#!/bin/sh\n[ \"$1\" = api ] || exit 2\nendpoint=$(printf '%s' \"$2\" | tr / +)\nexec /bin/cat \"$RHOST_GH_FIXTURES/$endpoint\"\n",
        )
        .map_err(|e| e.to_string())?;
        std::fs::set_permissions(gh, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let run = json!({"id":5,"head_sha":"a".repeat(40),"path":".github/workflows/release.yml","event":"workflow_dispatch","run_attempt":1,"status":"completed","conclusion":"success","repository":{"full_name":"starfield17/rhost"},"head_repository":{"full_name":"starfield17/rhost"}});
        let names = [
            "rhost-darwin_amd64",
            "rhost-darwin_arm64",
            "rhost-linux_amd64",
            "rhost-linux_arm64",
            "helper-linux-x86_64",
            "helper-linux-aarch64",
        ];
        let entries: Vec<Value> = names.iter().enumerate().map(|(i,name)| json!({"id":10+i,"name":name,"expired":false,"size_in_bytes":1,"digest":format!("sha256:{}","0".repeat(64)),"workflow_run":{"id":5}})).collect();
        let mut f = Self {
            root,
            run,
            artifacts: json!({"total_count":6,"artifacts":entries}),
        };
        let asset = "rhost_5.0.0_darwin_arm64";
        let bytes = b"the exact candidate bytes";
        let sha = hash(bytes);
        std::fs::write(f.root.join("files").join(asset), bytes).map_err(|e| e.to_string())?;
        std::fs::write(
            f.root.join("files").join(format!("{asset}.sha256")),
            format!("{sha}  {asset}\n"),
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(f.root.join("files/candidate-darwin_arm64.json"), json!({"format":1,"commit":"a".repeat(40),"version":"5.0.0","platform":"darwin_arm64","sha256":sha}).to_string()).map_err(|e| e.to_string())?;
        f.pack()?;
        Ok(f)
    }
    fn reply(&self, endpoint: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.root.join("replies").join(format!(
            "repos+starfield17+rhost+{}",
            endpoint.replace('/', "+")
        ));
        std::fs::create_dir_all(path.parent().ok_or("reply parent")?).map_err(|e| e.to_string())?;
        std::fs::write(path, bytes).map_err(|e| e.to_string())
    }
    fn pack(&mut self) -> Result<(), String> {
        let path = self.root.join("candidate.zip");
        let _ = std::fs::remove_file(&path);
        let status = Command::new("zip")
            .current_dir(self.root.join("files"))
            .arg("-q")
            .arg(&path)
            .args([
                "rhost_5.0.0_darwin_arm64",
                "rhost_5.0.0_darwin_arm64.sha256",
                "candidate-darwin_arm64.json",
            ])
            .status()
            .map_err(|e| e.to_string())?;
        assert!(status.success());
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        self.artifacts["artifacts"][1]["digest"] = format!("sha256:{}", hash(&bytes)).into();
        self.artifacts["artifacts"][1]["size_in_bytes"] = bytes.len().into();
        self.reply("actions/artifacts/11/zip", &bytes)
    }
    fn invoke(&self) -> Result<Output, String> {
        self.reply("actions/runs/5", self.run.to_string().as_bytes())?;
        self.reply(
            "actions/runs/5/artifacts?per_page=100",
            self.artifacts.to_string().as_bytes(),
        )?;
        Command::new("python3")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/release_api.py"))
            .args([
                "--run",
                "5",
                "--commit",
                &"a".repeat(40),
                "--version",
                "5.0.0",
                "--platform",
                "darwin_arm64",
            ])
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
            .output()
            .map_err(|e| e.to_string())
    }
    fn refused(&self, reason: &str) -> Result<(), String> {
        let out = self.invoke()?;
        assert!(
            !out.status.success(),
            "accepted invalid candidate: {reason}"
        );
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
fn exact_archive_and_contained_binary_have_separate_identities() -> Result<(), String> {
    let mut f = Fixture::new()?;
    let good = f.invoke()?;
    assert!(good.status.success(), "{good:?}");
    let record: Value = serde_json::from_slice(&good.stdout).map_err(|e| e.to_string())?;
    assert_eq!(record["artifact_id"], 11);
    assert_eq!(record["sha256"], hash(b"the exact candidate bytes"));
    assert_ne!(record["sha256"], record["archive_sha256"]);
    std::fs::write(
        f.root.join("files/rhost_5.0.0_darwin_arm64"),
        b"replacement",
    )
    .map_err(|e| e.to_string())?;
    f.pack()?;
    f.refused("client record mismatch")
}

#[test]
fn changed_attempts_sources_and_incomplete_platforms_cannot_be_promoted() -> Result<(), String> {
    let mut f = Fixture::new()?;
    f.run["run_attempt"] = 2.into();
    f.refused("only first-attempt")?;
    f.run["run_attempt"] = 1.into();
    f.run["head_sha"] = "b".repeat(40).into();
    f.refused("run source/workflow mismatch")?;
    f.run["head_sha"] = "a".repeat(40).into();
    f.artifacts["artifacts"][1]["expired"] = true.into();
    f.refused("artifact expired")?;
    f.artifacts["artifacts"][1]["expired"] = false.into();
    f.artifacts["artifacts"][1]["workflow_run"]["id"] = 6.into();
    f.refused("artifact belongs to another run")?;
    f.artifacts["artifacts"][1]["workflow_run"]["id"] = 5.into();
    f.artifacts["artifacts"][5]["name"] = "unexpected".into();
    f.refused("incomplete candidate platforms")
}
