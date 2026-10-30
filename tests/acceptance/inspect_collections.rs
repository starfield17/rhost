//! Discovery is separate from content observations and continuations are bounded.
use crate::{
    remote_fs_support::{ask, create, sha256_hex},
    support::Harness,
};
use serde_json::json;

#[test]
fn exclusions_precede_budgets_and_find_never_reads_file_contents() -> Result<(), String> {
    let h = Harness::open("find-exclusions")?;
    create(&h, "tree/source/a.rs", b"needle\n")?;
    std::fs::create_dir_all(h.path("tree/generated")).map_err(|e| e.to_string())?;
    for n in 0..4200 {
        std::fs::write(h.path(&format!("tree/generated/{n}")), [0xff])
            .map_err(|e| e.to_string())?;
    }
    let page = ask(
        &h,
        json!({"op":"find","root":h.path("tree"),"exclude_dirs":["generated"],"max_results":128}),
    )?;
    assert_eq!(page["incomplete"], false, "{page}");
    assert_eq!(page["items"].as_array().ok_or("items")?.len(), 1);
    assert_eq!(page["scanned_bytes"], 0);
    assert_eq!(page["next_cursor"], serde_json::Value::Null);
    Ok(())
}

#[test]
fn continuation_reads_new_contents_without_reopening_prior_files() -> Result<(), String> {
    let h = Harness::open("search-collection-pages")?;
    let a = create(&h, "tree/a", b"needle first\n")?;
    let b = create(&h, "tree/b", b"needle before\n")?;
    let mut q = json!({"op":"search","root":h.path("tree"),"pattern":"needle","max_results":1});
    let first = ask(&h, q.clone())?;
    assert_eq!(first["items"][0]["path"], a, "{first}");
    q["cursor"] = first["next_cursor"].clone();
    std::fs::remove_file(&a).map_err(|e| e.to_string())?;
    std::fs::write(&b, b"needle after\n").map_err(|e| e.to_string())?;
    create(&h, "tree/new", b"needle new\n")?;
    let next = ask(&h, q)?;
    assert_eq!(next["items"].as_array().ok_or("items")?.len(), 1, "{next}");
    assert_eq!(next["items"][0]["path"], b);
    assert_eq!(next["items"][0]["sha256"], sha256_hex(b"needle after\n"));
    assert_eq!(next["scanned_files"], 1);
    assert_eq!(next["scanned_bytes"], 13);
    assert_eq!(next["incomplete"], false);
    assert!(next["next_cursor"].is_null());
    Ok(())
}

#[test]
fn cursor_boundaries_and_expired_collections_never_silently_restart() -> Result<(), String> {
    let h = Harness::open("collection-expiry")?;
    create(&h, "tree/a", b"needle\n")?;
    create(&h, "tree/b", b"needle\n")?;
    let mut q = json!({"op":"search","root":h.path("tree"),"pattern":"needle","max_results":1,"collection_ttl_seconds":1});
    let first = ask(&h, q.clone())?;
    let cursor = first["next_cursor"].as_str().ok_or("cursor")?;
    let (prefix, offset) = cursor.rsplit_once(':').ok_or("offset")?;
    q["cursor"] = format!(
        "{prefix}:{}",
        offset.parse::<u64>().map_err(|e| e.to_string())? + 1
    )
    .into();
    assert_eq!(ask(&h, q.clone())?["error"], "CONFIG_INVALID");
    let expiry = first["collection"]["expires_unix"]
        .as_u64()
        .ok_or("expiry")?;
    std::fs::write(
        h.path("home/.local/state/rhost/v5/clock"),
        expiry.to_be_bytes(),
    )
    .map_err(|e| e.to_string())?;
    q["cursor"] = cursor.into();
    assert_eq!(ask(&h, q)?["error"], "QUERY_EXPIRED");
    Ok(())
}

#[test]
fn corrupted_path_records_and_indexes_are_refused_before_content_reads() -> Result<(), String> {
    let h = Harness::open("collection-corruption")?;
    create(&h, "tree/a", b"needle\n")?;
    create(&h, "tree/b", b"needle\n")?;
    let mut q = json!({"op":"search","root":h.path("tree"),"pattern":"needle","max_results":1});
    let first = ask(&h, q.clone())?;
    q["cursor"] = first["next_cursor"].clone();
    let id = first["collection"]["id"].as_str().ok_or("id")?;
    let directory = h.path(&format!("home/.local/state/rhost/v5/queries/{id}"));
    let manifest = directory.join("paths");
    let original = std::fs::read(&manifest).map_err(|e| e.to_string())?;
    let mut bytes = original.clone();
    let offset = first["next_cursor"]
        .as_str()
        .and_then(|s| s.rsplit(':').next())
        .ok_or("offset")?
        .parse::<usize>()
        .map_err(|e| e.to_string())?;
    bytes[offset + 4] ^= 1;
    std::fs::write(&manifest, bytes).map_err(|e| e.to_string())?;
    assert_eq!(ask(&h, q.clone())?["error"], "STATE_UNAVAILABLE");
    std::fs::write(&manifest, original).map_err(|e| e.to_string())?;
    let index = directory.join("paths.index");
    let mut bytes = std::fs::read(&index).map_err(|e| e.to_string())?;
    bytes[15] ^= 1;
    std::fs::write(index, bytes).map_err(|e| e.to_string())?;
    let refused = ask(&h, q)?;
    assert!(
        matches!(
            refused["error"].as_str(),
            Some("CONFIG_INVALID" | "STATE_UNAVAILABLE")
        ),
        "{refused}"
    );
    Ok(())
}
