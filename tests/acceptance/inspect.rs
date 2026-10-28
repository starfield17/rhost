//! Structured search/read-many return revisions, without interpreting shell output.
use crate::{
    remote_fs_support::{ask, create, sha256_hex},
    support::Harness,
};
use serde_json::json;
#[test]
fn search_returns_file_hashes_and_refuses_a_changed_continuation() -> Result<(), String> {
    let h = Harness::open("native-search")?;
    let a = create(&h, "tree/a.rs", b"one\nneedle here\n")?;
    let b = create(&h, "tree/b.rs", b"needle too\n")?;
    std::os::unix::fs::symlink(&a, h.path("tree/link.rs")).map_err(|e| e.to_string())?;
    let mut q = json!({"op":"search","root":h.path("tree"),"glob":"*.rs","pattern":"needle","max_results":1,"max_matches":10,"max_bytes":4096});
    let found = ask(&h, q.clone())?;
    assert_eq!(found["items"][0]["path"], a, "{found}");
    assert_eq!(
        found["items"][0]["sha256"],
        sha256_hex(b"one\nneedle here\n")
    );
    assert_eq!(found["items"][0]["matches"][0]["line"], 2);
    assert!(found["next_cursor"].is_string(), "{found}");
    q["cursor"] = found["next_cursor"].clone();
    let next = ask(&h, q.clone())?;
    assert_eq!(next["items"][0]["path"], b, "{next}");
    assert!(next["next_cursor"].is_null());
    std::fs::write(&b, b"changed\n").map_err(|e| e.to_string())?;
    assert_eq!(ask(&h, q)?["error"], "FILE_CONFLICT");
    Ok(())
}
#[test]
fn read_many_has_an_aggregate_content_budget_and_per_file_refusals() -> Result<(), String> {
    let h = Harness::open("native-read-many")?;
    let a = create(&h, "a", b"alpha\n")?;
    let b = create(&h, "b", b"beta\n")?;
    let missing = h.path("missing");
    let read = ask(
        &h,
        json!({"op":"read_many","paths":[missing,a,b],"start":1,"lines":20,"max_bytes":6}),
    )?;
    assert_eq!(
        read["items"][0]["error"]["code"], "FILE_NOT_FOUND",
        "{read}"
    );
    assert_eq!(read["items"][1]["data"]["content"], "alpha\n");
    assert_eq!(read["unread"][0], b);
    Ok(())
}

#[test]
fn per_file_match_truncation_and_scan_incompleteness_are_distinct() -> Result<(), String> {
    let h = Harness::open("search-coverage")?;
    create(&h, "tree/a", b"needle\nneedle\n")?;
    create(&h, "tree/b", b"needle\n")?;
    let q = json!({"op":"search","root":h.path("tree"),"pattern":"needle","max_results":1,"max_matches":1,"max_bytes":1024});
    let page = ask(&h, q.clone())?;
    assert_eq!(page["items"][0]["matches_truncated"], true, "{page}");
    assert_eq!(page["incomplete"], false);
    assert!(page["next_cursor"].is_string());
    create(&h, "tree/binary", &[0xff, 0xfe])?;
    let partial = ask(&h, q)?;
    assert_eq!(partial["incomplete"], true, "{partial}");
    assert!(partial["next_cursor"].is_null());
    assert_eq!(partial["skipped"][0]["code"], "INVALID_TEXT");
    Ok(())
}
