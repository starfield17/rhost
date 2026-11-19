//! Publication requires the complete inventory with exact binary checksums.
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);
const VERSION: &str = "5.0.0";
const ASSETS: [&str; 6] = [
    "rhost_5.0.0_darwin_amd64",
    "rhost_5.0.0_darwin_arm64",
    "rhost_5.0.0_linux_amd64",
    "rhost_5.0.0_linux_arm64",
    "rhost-helper_5.0.0_linux-x86_64",
    "rhost-helper_5.0.0_linux-aarch64",
];

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> std::io::Result<Self> {
        let root = Self(std::env::temp_dir().join(format!(
            "rhost-release-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        std::fs::create_dir(&root.0)?;
        for asset in ASSETS {
            root.write(asset, asset)?;
            root.write_sidecar(asset)?;
        }
        Ok(root)
    }

    fn write(&self, name: &str, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
        std::fs::write(self.0.join(name), contents)
    }

    fn write_sidecar(&self, asset: &str) -> std::io::Result<()> {
        let digest: String = Sha256::digest(asset.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.write(&format!("{asset}.sha256"), format!("{digest}  {asset}\n"))
    }

    fn verify(&self) -> std::io::Result<Output> {
        Command::new("sh")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/scripts/verify-release-assets.sh"
            ))
            .arg(VERSION)
            .arg(&self.0)
            .output()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn all_six_native_assets_and_their_sidecars_pass_publication_verification() -> std::io::Result<()> {
    let fixture = Fixture::new()?;
    let result = fixture.verify()?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}

#[test]
fn incomplete_or_substituted_release_assets_fail_closed() -> std::io::Result<()> {
    let fixture = Fixture::new()?;
    let asset = ASSETS[4];
    let sidecar = format!("{asset}.sha256");
    fixture.write(asset, "substituted bytes")?;
    assert!(!fixture.verify()?.status.success());
    fixture.write(asset, asset)?;
    fixture.write(&sidecar, "malformed checksum")?;
    assert!(!fixture.verify()?.status.success());
    std::fs::copy(
        fixture.0.join(format!("{}.sha256", ASSETS[5])),
        fixture.0.join(&sidecar),
    )?;
    assert!(!fixture.verify()?.status.success());
    std::fs::remove_file(fixture.0.join(sidecar))?;
    assert!(!fixture.verify()?.status.success());
    fixture.write_sidecar(asset)?;
    fixture.write("unexpected", "an extra file")?;
    assert!(!fixture.verify()?.status.success());
    Ok(())
}
