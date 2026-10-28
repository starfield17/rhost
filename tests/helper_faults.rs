//! Local sanitizer companion; exact shipping-byte verification is a separate gate.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};
#[test]
fn each_owned_allocation_may_fail_without_invalid_memory_access()
-> Result<(), Box<dyn std::error::Error>> {
    let binary = std::env::var_os("RHOST_HELPER_FAULT_BIN")
        .ok_or("set RHOST_HELPER_FAULT_BIN to the local instrumented helper")?;
    let root =
        std::fs::canonicalize("target")?.join(format!("helper-alloc-fault-{}", std::process::id()));
    let hash = format!("{:x}", Sha256::digest(b"old\n"));
    let generation = "a".repeat(32);
    for op in ["search", "read_many", "changeset.apply"] {
        for fail in 1..=600 {
            if root.exists() {
                std::fs::remove_dir_all(&root)?;
            }
            std::fs::create_dir_all(root.join("files"))?;
            let state = root.join("state");
            std::fs::create_dir(&state)?;
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))?;
            std::fs::create_dir(state.join("runs"))?;
            std::fs::set_permissions(state.join("runs"), std::fs::Permissions::from_mode(0o700))?;
            for (name, bytes) in [
                ("generation", generation.as_bytes()),
                ("registry.lock", b"".as_slice()),
            ] {
                std::fs::write(state.join(name), bytes)?;
                std::fs::set_permissions(state.join(name), std::fs::Permissions::from_mode(0o600))?;
            }
            let a = root.join("files/a");
            std::fs::write(&a, b"old\n")?;
            let expiry = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
                + 3600;
            let request = match op {
                "search" => json!({"op":op,"root":root.join("files"),"pattern":"old"}),
                "read_many" => json!({"op":op,"paths":[a,root.join("missing")]}),
                _ => {
                    json!({"op":op,"generation":generation,"key":format!("{expiry}-{}","b".repeat(64)),"expires_unix":expiry,"changes":[{"op":"write","path":a,"if_hash":hash,"content":"bmV3Cg=="},{"op":"write","path":root.join("files/b"),"content":"eAo="}]})
                }
            };
            let mut child = Command::new(&binary)
                .arg("--files-json")
                .env("RHOST_REMOTE_STATE", &state)
                .env_remove("RHOST_EXPECT_BOOT_ID")
                .env("RHOST_FAIL_ALLOC_AT", fail.to_string())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            child
                .stdin
                .take()
                .ok_or("stdin")?
                .write_all(&serde_json::to_vec(&request)?)?;
            let result = child.wait_with_output()?;
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                matches!(result.status.code(), Some(0 | 255)),
                "{op} allocation {fail}: {} {stderr}",
                result.status
            );
            assert!(
                !stderr.contains("Sanitizer") && !stderr.contains("runtime error"),
                "{op} allocation {fail}: {stderr}"
            );
            if !result.stdout.is_empty() {
                let _: serde_json::Value = serde_json::from_slice(&result.stdout)?;
            }
        }
    }
    std::fs::remove_dir_all(root)?;
    Ok(())
}
