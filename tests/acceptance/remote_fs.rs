//! Direct conformance for the embedded remote filesystem helper.
//!
//! These cases deliberately stop below the CLI: one JSON request goes to the
//! exact `PROGRAM` embedded in the binary, and one JSON answer comes back. That
//! makes filesystem edge failures diagnosable without a round trip through a
//! stubbed SSH transport. The CLI acceptance suite remains the oracle for the
//! public envelope; this oracle proves the helper's local semantics.

use crate::remote_fs_support::{
    ALPHA, GAMMA, X, ask, assert_no_write_temp, create, expect_error, mode, path_text, read_bytes,
    sha256_hex,
};
use crate::support::{Harness, fail, python3_available};
use serde_json::{Value, json};
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn request_envelopes_are_checked_before_the_helper_touches_a_file() -> Result<(), String> {
    let harness = Harness::open("fs-helper-request")?;
    if !python3_available() {
        return Ok(());
    }
    let cases: [(Value, &str); 4] = [
        (json!(["read"]), "CONFIG_INVALID"),
        (json!({"op": "read", "max_bytes": 1024}), "CONFIG_INVALID"),
        (
            json!({"op": "unknown", "path": "/tmp/x", "max_bytes": 1024}),
            "CONFIG_INVALID",
        ),
        (
            json!({"op": "read", "path": "/tmp/x", "max_bytes": 0}),
            "CONFIG_INVALID",
        ),
    ];
    for (request, wanted) in cases {
        expect_error(&harness, request, wanted)?;
    }
    Ok(())
}

#[test]
fn read_pages_bounded_utf8_text_with_whole_file_identity() -> Result<(), String> {
    let harness = Harness::open("fs-helper-read")?;
    if !python3_available() {
        return Ok(());
    }
    let text = create(&harness, "read/text.txt", b"alpha\nbeta\n")?;
    let answer = ask(
        &harness,
        json!({"op": "read", "path": text, "start": 1, "lines": 200, "max_bytes": 256 * 1024}),
    )?;
    assert_eq!(answer["content"], json!("alpha\nbeta\n"));
    assert_eq!(answer["lines"], json!(2));
    assert_eq!(answer["total_lines"], json!(2));
    assert_eq!(answer["truncated"], json!(false));
    assert_eq!(
        answer["sha256"].as_str().map(str::len),
        Some(64),
        "{answer}"
    );

    let bounded = ask(
        &harness,
        json!({"op": "read", "path": text, "start": 1, "lines": 200, "max_bytes": 6}),
    )?;
    assert_eq!(bounded["content"], json!("alpha\n"));
    assert_eq!(bounded["lines"], json!(1));
    assert_eq!(bounded["total_lines"], json!(2));
    assert_eq!(bounded["truncated"], json!(true));

    let page = ask(
        &harness,
        json!({"op": "read", "path": text, "start": 2, "lines": 1, "max_bytes": 256 * 1024}),
    )?;
    assert_eq!(page["content"], json!("beta\n"));
    assert_eq!(page["start"], json!(2));

    let no_newline = create(&harness, "read/no-newline", b"one\ntwo\nthree")?;
    let tail = ask(
        &harness,
        json!({"op": "read", "path": no_newline, "start": 3, "lines": 1, "max_bytes": 256 * 1024}),
    )?;
    assert_eq!(tail["content"], json!("three"));
    assert_eq!(tail["total_lines"], json!(3));

    expect_error(
        &harness,
        json!({"op": "read", "path": path_text(&harness, "read"), "max_bytes": 1024}),
        "INVALID_TARGET",
    )?;
    expect_error(
        &harness,
        json!({"op": "read", "path": path_text(&harness, "read/missing"), "max_bytes": 1024}),
        "FILE_NOT_FOUND",
    )?;
    let invalid = create(&harness, "read/invalid-utf8", b"\xff\n")?;
    expect_error(
        &harness,
        json!({"op": "read", "path": invalid, "max_bytes": 1024}),
        "INVALID_TEXT",
    )
}

#[test]
fn writes_are_hash_guarded_permission_explicit_and_clean_on_failure() -> Result<(), String> {
    let harness = Harness::open("fs-helper-write")?;
    if !python3_available() {
        return Ok(());
    }
    let created = harness.path("create/nested/new.txt");
    let created_text = created.to_string_lossy().into_owned();
    let answer = ask(
        &harness,
        json!({
            "op": "write",
            "path": created_text,
            "content": ALPHA,
            "parents": true,
            "max_bytes": 256 * 1024
        }),
    )?;
    assert_eq!(answer["bytes"], json!(6));
    assert_eq!(read_bytes(&created_text)?, b"alpha\n");
    assert_eq!(mode(&created_text)?, 0o600);

    let replaced_path = create(&harness, "mode/preserved.txt", b"alpha\n")?;
    std::fs::set_permissions(&replaced_path, std::fs::Permissions::from_mode(0o640))
        .map_err(|error| fail("chmod fixture", error))?;
    let hash = ask(
        &harness,
        json!({"op": "read", "path": replaced_path, "max_bytes": 256 * 1024}),
    )?["sha256"]
        .as_str()
        .ok_or_else(|| "read returned no hash".to_string())?
        .to_string();
    ask(
        &harness,
        json!({
            "op": "write", "path": replaced_path, "content": GAMMA,
            "if_hash": hash, "max_bytes": 256 * 1024
        }),
    )?;
    assert_eq!(read_bytes(&replaced_path)?, b"gamma\n");
    assert_eq!(mode(&replaced_path)?, 0o640);

    let explicit_path = create(&harness, "mode/explicit.txt", b"alpha\n")?;
    let explicit_hash = ask(
        &harness,
        json!({"op": "read", "path": explicit_path, "max_bytes": 256 * 1024}),
    )?["sha256"]
        .as_str()
        .ok_or_else(|| "read returned no hash".to_string())?
        .to_string();
    ask(
        &harness,
        json!({
            "op": "write", "path": explicit_path, "content": GAMMA,
            "if_hash": explicit_hash, "file_mode": "0644", "max_bytes": 256 * 1024
        }),
    )?;
    assert_eq!(mode(&explicit_path)?, 0o644);

    expect_error(
        &harness,
        json!({
            "op": "write", "path": explicit_path, "content": GAMMA,
            "if_hash": "stale", "max_bytes": 256 * 1024
        }),
        "FILE_CONFLICT",
    )?;
    expect_error(
        &harness,
        json!({
            "op": "write", "path": explicit_path, "content": GAMMA,
            "max_bytes": 256 * 1024
        }),
        "HASH_REQUIRED",
    )?;
    assert_no_write_temp(harness.path("mode").as_path(), "a refused write")?;

    let bad_mode_parent = harness.path("bad-mode");
    std::fs::create_dir_all(&bad_mode_parent)
        .map_err(|error| fail("prepare bad-mode fixture", error))?;
    let bad_mode = bad_mode_parent.join("file.txt");
    expect_error(
        &harness,
        json!({
            "op": "write", "path": bad_mode, "content": X, "file_mode": "0899",
            "max_bytes": 256 * 1024
        }),
        "CONFIG_INVALID",
    )?;
    assert!(
        !bad_mode.exists(),
        "an invalid mode must not create the named file"
    );
    assert_no_write_temp(&bad_mode_parent, "an invalid mode")?;
    expect_error(
        &harness,
        json!({
            "op": "write", "path": path_text(&harness, "missing-parent/file"),
            "content": X, "max_bytes": 256 * 1024
        }),
        "FILE_NOT_FOUND",
    )?;
    expect_error(
        &harness,
        json!({
            "op": "write", "path": path_text(&harness, "bad-base64/file"), "content": "not!",
            "parents": true, "max_bytes": 256 * 1024
        }),
        "CONFIG_INVALID",
    )?;
    expect_error(
        &harness,
        json!({
            "op": "write", "path": path_text(&harness, "non-utf8/file"), "content": "/w==",
            "parents": true, "max_bytes": 256 * 1024
        }),
        "INVALID_TEXT",
    )
}

#[test]
fn patches_apply_once_and_refuse_every_shape_that_cannot_be_trusted() -> Result<(), String> {
    let harness = Harness::open("fs-helper-patch")?;
    if !python3_available() {
        return Ok(());
    }
    let path = create(&harness, "patch/file.txt", b"a\nb\nc\nd\n")?;
    let hash = ask(
        &harness,
        json!({"op": "read", "path": path, "max_bytes": 256 * 1024}),
    )?["sha256"]
        .as_str()
        .ok_or_else(|| "read returned no hash".to_string())?
        .to_string();
    ask(
        &harness,
        json!({
            "op": "patch", "path": path, "if_hash": hash, "max_bytes": 256 * 1024,
            "edits": [
                {"start": 3, "end": 3, "text": "late\n"},
                {"start": 1, "end": 1, "text": "early\n"}
            ]
        }),
    )?;
    assert_eq!(read_bytes(&path)?, b"early\nb\nlate\nd\n");

    let current = ask(
        &harness,
        json!({"op": "read", "path": path, "max_bytes": 256 * 1024}),
    )?["sha256"]
        .as_str()
        .ok_or_else(|| "read returned no hash".to_string())?
        .to_string();
    let cases: [(&str, Value, &str); 5] = [
        ("empty", json!([]), "INVALID_PATCH"),
        (
            "out of range",
            json!([{"start": 1, "end": 5, "text": "x"}]),
            "INVALID_PATCH",
        ),
        (
            "overlap",
            json!([
                {"start": 1, "end": 2, "text": "x"},
                {"start": 2, "end": 3, "text": "y"}
            ]),
            "INVALID_PATCH",
        ),
        (
            "missing text",
            json!([{"start": 1, "end": 1}]),
            "INVALID_PATCH",
        ),
        (
            "extra field",
            json!([{"start": 1, "end": 1, "text": "x", "surprise": 1}]),
            "INVALID_PATCH",
        ),
    ];
    for (_, edits, wanted) in cases {
        expect_error(
            &harness,
            json!({
                "op": "patch", "path": path, "if_hash": current, "max_bytes": 256 * 1024,
                "edits": edits
            }),
            wanted,
        )?;
    }
    assert_eq!(read_bytes(&path)?, b"early\nb\nlate\nd\n");
    assert_no_write_temp(harness.path("patch").as_path(), "a refused patch")?;

    let source = std::fs::read(&path).map_err(|error| fail("copy fixture", error))?;
    let stale = create(&harness, "patch/stale.txt", &source)?;
    expect_error(
        &harness,
        json!({
            "op": "patch", "path": stale, "if_hash": "stale",
            "max_bytes": 256 * 1024, "edits": [{"start":1,"end":1,"text":"x\n"}]
        }),
        "FILE_CONFLICT",
    )?;
    let invalid = create(&harness, "patch/invalid-utf8", b"\xff\n")?;
    expect_error(
        &harness,
        json!({"op": "read", "path": invalid, "max_bytes": 256 * 1024}),
        "INVALID_TEXT",
    )?;
    let invalid_hash = sha256_hex(b"\xff\n");
    expect_error(
        &harness,
        json!({
            "op": "patch", "path": invalid, "if_hash": invalid_hash,
            "max_bytes": 256 * 1024, "edits": [{"start":1,"end":1,"text":"x\n"}]
        }),
        "INVALID_TEXT",
    )
}

#[test]
fn editing_refuses_a_symlinked_target_or_parent_without_following_it() -> Result<(), String> {
    let harness = Harness::open("fs-helper-symlink")?;
    if !python3_available() {
        return Ok(());
    }
    let real = create(&harness, "link/real.txt", b"target\n")?;
    let link = harness.path("link/final-link");
    symlink(&real, &link).map_err(|error| fail("create symlink", error))?;
    let link = link.to_string_lossy().into_owned();
    let write = |target: &str| {
        json!({
            "op": "write", "path": target, "content": X,
            "if_hash": "a".repeat(64), "max_bytes": 256 * 1024
        })
    };
    for request in [
        json!({"op": "read", "path": link, "max_bytes": 256 * 1024}),
        write(&link),
        json!({
            "op": "patch", "path": link, "if_hash": "a".repeat(64),
            "max_bytes": 256 * 1024, "edits": [{"start": 1, "end": 1, "text": "nope\n"}]
        }),
    ] {
        expect_error(&harness, request, "INVALID_TARGET")?;
    }
    assert_eq!(read_bytes(&real)?, b"target\n");
    assert_no_write_temp(harness.path("link").as_path(), "a refused symlinked target")?;

    let real_dir = harness.path("parent-target");
    std::fs::create_dir_all(&real_dir).map_err(|error| fail("prepare parent", error))?;
    let nested = real_dir.join("nested.txt");
    std::fs::write(&nested, b"nested\n").map_err(|error| fail("write nested", error))?;
    let parent_link = harness.path("parent-link");
    symlink(&real_dir, &parent_link).map_err(|error| fail("create directory link", error))?;
    let nested_link = parent_link.join("nested.txt");
    let nested_link = nested_link.to_string_lossy().into_owned();
    for request in [
        json!({"op": "read", "path": nested_link, "max_bytes": 256 * 1024}),
        write(&nested_link),
        json!({
            "op": "patch", "path": nested_link, "if_hash": "a".repeat(64),
            "max_bytes": 256 * 1024, "edits": [{"start": 1, "end": 1, "text": "nope\n"}]
        }),
    ] {
        expect_error(&harness, request, "INVALID_TARGET")?;
    }
    assert_eq!(read_bytes(&nested.to_string_lossy())?, b"nested\n");
    assert_no_write_temp(harness.path(".").as_path(), "a refused symlinked parent")?;
    Ok(())
}

#[test]
fn parents_are_created_only_when_no_symlink_shadows_a_component() -> Result<(), String> {
    let harness = Harness::open("fs-helper-parents")?;
    if !python3_available() {
        return Ok(());
    }
    let real = harness.path("real-parent");
    std::fs::create_dir_all(&real).map_err(|error| fail("prepare parent", error))?;
    let link = harness.path("link-parent");
    symlink(&real, &link).map_err(|error| fail("create parent link", error))?;
    let target = link.join("created.txt");
    let target = target.to_string_lossy().into_owned();
    expect_error(
        &harness,
        json!({
            "op": "write", "path": target, "content": X, "parents": true,
            "max_bytes": 256 * 1024
        }),
        "INVALID_TARGET",
    )?;
    assert!(
        !real.join("created.txt").exists(),
        "parents must not be written through a symlinked component"
    );
    Ok(())
}

#[test]
fn destructive_sync_resolution_follows_the_authorized_target_only() -> Result<(), String> {
    let harness = Harness::open("fs-helper-resolve")?;
    if !python3_available() {
        return Ok(());
    }
    let safe = harness.path("sync/tree");
    std::fs::create_dir_all(&safe).map_err(|error| fail("prepare sync fixture", error))?;
    let safe = safe.to_string_lossy().into_owned();
    let canonical = std::fs::canonicalize(&safe).map_err(|error| fail("canonicalize", error))?;
    let actual = ask(
        &harness,
        json!({"op": "resolve", "path": safe, "delete": false}),
    )?["path"]
        .as_str()
        .ok_or_else(|| "resolve returned no path".to_string())?
        .to_string();
    assert_eq!(actual, canonical.to_string_lossy());
    let destructive = ask(
        &harness,
        json!({"op": "resolve", "path": safe, "delete": true}),
    )?;
    assert_eq!(destructive["path"], json!(actual));

    for path in ["/".to_string(), "/usr".to_string(), "~".to_string()] {
        expect_error(
            &harness,
            json!({"op": "resolve", "path": path, "delete": true}),
            "SYNC_REJECTED",
        )?;
    }
    Ok(())
}
