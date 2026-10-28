//! Targeted changes, without a project/worktree service or automatic rollback.
use crate::{
    remote_fs_support::{ask, create, prepare_native, read_bytes, sha256_hex},
    support::Harness,
};
use serde_json::json;
#[test]
fn all_targets_pass_preflight_and_same_key_only_observes() -> Result<(), String> {
    let h = Harness::open("changeset-preflight")?;
    let (generation, time) = prepare_native(&h)?;
    let expiry = time + 3600;
    let a = create(&h, "files/a", b"old a\n")?;
    let b = create(&h, "files/b", b"old b\n")?;
    let key = format!("{expiry}-{}", "a".repeat(64));
    let mut request = json!({"op":"changeset.apply","generation":generation,"key":key,"expires_unix":expiry,"changes":[
      {"op":"write","path":a,"if_hash":sha256_hex(b"old a\n"),"content":"bmV3IGEK"},
      {"op":"write","path":b,"if_hash":sha256_hex(b"wrong"),"content":"bmV3IGIK"}]});
    let refused = ask(&h, request.clone())?;
    assert_eq!(refused["error"]["code"], "FILE_CONFLICT", "{refused}");
    assert_eq!(refused["record"]["mutation"], "not_applied");
    assert_eq!(read_bytes(&a)?, b"old a\n");
    assert_eq!(read_bytes(&b)?, b"old b\n");
    request["changes"][1]["if_hash"] = sha256_hex(b"old b\n").into();
    assert_eq!(ask(&h, request.clone())?["error"], "IDEMPOTENCY_CONFLICT");
    request["key"] = format!("{expiry}-{}", "b".repeat(64)).into();
    let done = ask(&h, request.clone())?;
    assert!(done["error"].is_null(), "{done}");
    assert_eq!(done["record"]["status"], "applied");
    assert_eq!(done["record"]["mutation"], "applied");
    assert_eq!(read_bytes(&a)?, b"new a\n");
    assert_eq!(read_bytes(&b)?, b"new b\n");
    std::fs::write(&a, b"external\n").map_err(|e| e.to_string())?;
    assert_eq!(ask(&h, request.clone())?["record"]["status"], "applied");
    let rollback = ask(
        &h,
        json!({"op":"changeset.recover","generation":generation,"id":request["key"],"action":"rollback"}),
    )?;
    assert_eq!(rollback["error"]["code"], "FILE_CONFLICT", "{rollback}");
    assert_eq!(read_bytes(&a)?, b"external\n");
    assert_eq!(read_bytes(&b)?, b"new b\n");
    Ok(())
}
#[test]
fn explicit_rollback_restores_bytes_and_removes_only_guarded_new_files() -> Result<(), String> {
    let h = Harness::open("changeset-rollback")?;
    let (generation, time) = prepare_native(&h)?;
    let expiry = time + 3600;
    let a = create(&h, "files/a", b"original\n")?;
    let b = h.path("files/new").to_string_lossy().into_owned();
    let id = format!("{expiry}-{}", "c".repeat(64));
    let applied = ask(
        &h,
        json!({"op":"changeset.apply","generation":generation,"key":id,"expires_unix":expiry,"changes":[
      {"op":"write","path":a,"if_hash":sha256_hex(b"original\n"),"content":"bmV3Cg=="},
      {"op":"write","path":b,"content":"eAo="}]}),
    )?;
    assert_eq!(applied["record"]["status"], "applied", "{applied}");
    let restored = ask(
        &h,
        json!({"op":"changeset.recover","generation":generation,"id":id,"action":"rollback"}),
    )?;
    assert!(restored["error"].is_null(), "{restored}");
    assert_eq!(restored["record"]["status"], "rolled_back");
    assert_eq!(restored["record"]["mutation"], "applied");
    assert_eq!(read_bytes(&a)?, b"original\n");
    assert!(!std::path::Path::new(&b).exists());
    let queried = ask(
        &h,
        json!({"op":"changeset.query","generation":generation,"id":id}),
    )?;
    assert_eq!(queried["record"]["status"], "rolled_back");
    Ok(())
}
#[test]
fn duplicate_directory_aliases_are_refused_before_target_changes() -> Result<(), String> {
    let h = Harness::open("changeset-alias")?;
    let (generation, time) = prepare_native(&h)?;
    let expiry = time + 3600;
    let a = create(&h, "files/a", b"original\n")?;
    let alias = h.path("files/../files/a");
    let result = ask(
        &h,
        json!({"op":"changeset.apply","generation":generation,"key":format!("{expiry}-{}","d".repeat(64)),"expires_unix":expiry,"changes":[
      {"op":"write","path":a,"if_hash":sha256_hex(b"original\n"),"content":"eAo="},
      {"op":"write","path":alias,"if_hash":sha256_hex(b"original\n"),"content":"eAo="}]}),
    )?;
    assert_eq!(result["error"]["code"], "CONFIG_INVALID", "{result}");
    assert_eq!(read_bytes(&a)?, b"original\n");
    Ok(())
}

#[test]
fn recovery_after_partial_preflight_keeps_a_reloadable_journal() -> Result<(), String> {
    let h = Harness::open("changeset-reprepare")?;
    let (generation, time) = prepare_native(&h)?;
    let expiry = time + 3600;
    let a = create(&h, "files/a", b"a")?;
    let b = create(&h, "files/b", b"b")?;
    let id = format!("{expiry}-{}", "e".repeat(64));
    let refused = ask(
        &h,
        json!({"op":"changeset.apply","generation":generation,"key":id,"expires_unix":expiry,"changes":[
      {"op":"write","path":a,"if_hash":sha256_hex(b"a"),"content":"eAo="},
      {"op":"write","path":b,"if_hash":sha256_hex(b"changed"),"content":"eAo="}]}),
    )?;
    assert_eq!(refused["record"]["status"], "refused");
    std::fs::write(&b, b"changed").map_err(|e| e.to_string())?;
    let resumed = ask(
        &h,
        json!({"op":"changeset.recover","generation":generation,"id":id,"action":"resume"}),
    )?;
    assert!(resumed["error"].is_null(), "{resumed}");
    assert_eq!(resumed["record"]["status"], "applied");
    assert_eq!(
        ask(
            &h,
            json!({"op":"changeset.query","generation":generation,"id":id})
        )?["record"]["status"],
        "applied"
    );
    let restored = ask(
        &h,
        json!({"op":"changeset.recover","generation":generation,"id":id,"action":"rollback"}),
    )?;
    assert!(restored["error"].is_null(), "{restored}");
    assert_eq!(read_bytes(&a)?, b"a");
    assert_eq!(read_bytes(&b)?, b"changed");
    Ok(())
}
#[test]
fn pruning_persists_expiry_and_never_reauthorizes_a_key() -> Result<(), String> {
    let h = Harness::open("changeset-expiry")?;
    let (generation, time) = prepare_native(&h)?;
    let expiry = time + 4;
    let path = create(&h, "files/a", b"a")?;
    let id = format!("{expiry}-{}", "f".repeat(64));
    let request = json!({"op":"changeset.apply","generation":generation,"key":id,"expires_unix":expiry,"changes":[{"op":"write","path":path,"if_hash":sha256_hex(b"a"),"content":"eAo="}]});
    assert_eq!(ask(&h, request.clone())?["record"]["status"], "applied");
    let clock = h.path("home/.local/state/rhost/v5/clock");
    let before = u64::from_be_bytes(
        std::fs::read(&clock)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "clock")?,
    );
    assert!(before < expiry);
    while std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs()
        < expiry
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        ask(
            &h,
            json!({"op":"changeset.query","generation":generation,"id":id})
        )?["record"]["status"],
        "applied"
    );
    let pruned = ask(
        &h,
        json!({"op":"changeset.prune","generation":generation,"id":id}),
    )?;
    assert_eq!(pruned["pruned"], true, "{pruned}");
    let after = u64::from_be_bytes(
        std::fs::read(&clock)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "clock")?,
    );
    assert!(after >= expiry);
    assert_eq!(ask(&h, request)?["error"], "REQUEST_EXPIRED");
    Ok(())
}
#[test]
fn interrupted_install_intent_never_claims_no_target_mutation() -> Result<(), String> {
    let h = Harness::open("changeset-intent-evidence")?;
    let (generation, time) = prepare_native(&h)?;
    let expiry = time + 3600;
    let path = create(&h, "files/a", b"a")?;
    let id = format!("{expiry}-{}", "1".repeat(64));
    assert_eq!(
        ask(
            &h,
            json!({"op":"changeset.apply","generation":generation,"key":id,"expires_unix":expiry,"changes":[{"op":"write","path":path,"if_hash":sha256_hex(b"a"),"content":"eAo="}]})
        )?["record"]["status"],
        "applied"
    );
    // Reconstruct a durable syscall-before-result-marker boundary from a completed
    // installation. No fault hook is added to the shipping helper.
    let manifest = h.path(&format!(
        "home/.local/state/rhost/v5/changes/{id}/manifest.json"
    ));
    let mut record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    record["status"] = "applying".into();
    record["files"][0]["state"] = "installing".into();
    record["files"][0]["ever_applied"] = false.into();
    std::fs::write(
        &manifest,
        serde_json::to_vec(&record).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let observed = ask(
        &h,
        json!({"op":"changeset.query","generation":generation,"id":id}),
    )?;
    assert_eq!(observed["record"]["mutation"], "unknown", "{observed}");
    let recovered = ask(
        &h,
        json!({"op":"changeset.recover","generation":generation,"id":id,"action":"rollback"}),
    )?;
    assert!(recovered["error"].is_null(), "{recovered}");
    assert_eq!(read_bytes(&path)?, b"a");
    Ok(())
}

#[test]
fn process_death_keeps_partial_evidence_and_retry_never_continues_writing() -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let h = Harness::open("changeset-process-death")?;
    let (generation, time) = prepare_native(&h)?;
    let expiry = time + 3600;
    let id = format!("{expiry}-{}", "5".repeat(64));
    create(&h, "files/keep", b"unrelated")?;
    let content = rhost::base64::encode(&vec![b'x'; 64 * 1024]);
    let changes = (0..64)
        .map(|i| json!({"op":"write","path":h.path(&format!("files/f{i:03}")),"content":content}))
        .collect::<Vec<_>>();
    let request = json!({"op":"changeset.apply","generation":generation,"key":id,"expires_unix":expiry,"changes":changes});
    let output = std::fs::File::create(h.path("result")).map_err(|e| e.to_string())?;
    let mut child =
        Command::new(std::env::var_os("RHOST_HELPER_BIN").ok_or("set RHOST_HELPER_BIN")?)
            .arg("--files-json")
            .env("HOME", h.path("home"))
            .env_remove("RHOST_REMOTE_STATE")
            .env_remove("RHOST_EXPECT_BOOT_ID")
            .stdin(Stdio::piped())
            .stdout(output)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("stdin")?
        .write_all(&serde_json::to_vec(&request).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut interrupted = false;
    while std::time::Instant::now() < deadline {
        if h.path("files/f000").exists() && !h.path("files/f063").exists() {
            child.kill().map_err(|e| e.to_string())?;
            interrupted = true;
            break;
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    if !interrupted {
        let _ = child.kill();
    }
    let _ = child.wait();
    assert!(
        interrupted,
        "could not interrupt a real multi-file application"
    );
    let observation = ask(
        &h,
        json!({"op":"changeset.query","generation":generation,"id":id}),
    )?;
    assert!(
        matches!(
            observation["record"]["mutation"].as_str(),
            Some("partial" | "unknown")
        ),
        "{observation}"
    );
    assert_eq!(observation["record"]["owner_active"], false);
    let last = h.path("files/f063").exists();
    let retried = ask(&h, request)?;
    assert_eq!(retried["record"]["files"], observation["record"]["files"]);
    assert_eq!(h.path("files/f063").exists(), last);
    assert_eq!(
        read_bytes(&h.path("files/keep").to_string_lossy())?,
        b"unrelated"
    );
    Ok(())
}
