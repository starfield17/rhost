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
