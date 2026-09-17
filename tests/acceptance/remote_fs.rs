//! Direct conformance for the embedded remote filesystem helper.
//!
//! These cases deliberately stop below the CLI: one JSON request goes to the
//! exact `PROGRAM` embedded in the binary, and one JSON answer comes back. That
//! makes filesystem edge failures diagnosable without a round trip through a
//! stubbed SSH transport. The CLI acceptance suite remains the oracle for the
//! public envelope; this oracle proves the helper's local semantics.

use crate::support::{Harness, fail, python3_available};
use rhost::files::backend::PROGRAM;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

const ALPHA: &str = "YWxwaGEK";
const GAMMA: &str = "Z2FtbWEK";
const X: &str = "eAo=";

fn ask(harness: &Harness, request: Value) -> Result<Value, String> {
    let payload = serde_json::to_vec(&request).map_err(|error| fail("encode request", error))?;
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(PROGRAM)
        .env("HOME", harness.path("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| fail("spawn embedded helper", error))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "the helper has no stdin".to_string())?;
        stdin
            .write_all(&payload)
            .map_err(|error| fail("send request", error))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| fail("wait for helper", error))?;
    let status = output
        .status
        .code()
        .ok_or_else(|| "the helper exited by signal".to_string())?;
    if status != 0 {
        return Err(format!(
            "the helper exited {status}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        fail(
            "helper stdout is not one JSON answer",
            format!(
                "{error}; stderr={}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    })
}

fn expect_error(harness: &Harness, request: Value, wanted: &str) -> Result<(), String> {
    let answer = ask(harness, request)?;
    if answer.get("error").and_then(Value::as_str) == Some(wanted) {
        return Ok(());
    }
    Err(format!("expected {wanted}, got {answer}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn create(harness: &Harness, relative: &str, body: &[u8]) -> Result<String, String> {
    let path = harness.path(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| fail("prepare fixture", error))?;
    }
    std::fs::write(&path, body).map_err(|error| fail("write fixture", error))?;
    Ok(path.to_string_lossy().into_owned())
}

fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| fail("read result", error))
}

fn mode(path: &str) -> Result<u32, String> {
    let metadata = std::fs::metadata(path).map_err(|error| fail("stat result", error))?;
    Ok(metadata.permissions().mode() & 0o777)
}

fn path_text(harness: &Harness, relative: &str) -> String {
    harness.path(relative).to_string_lossy().into_owned()
}

fn assert_no_write_temp(parent: &Path, context: &str) -> Result<(), String> {
    for entry in std::fs::read_dir(parent).map_err(|error| fail("inspect parent", error))? {
        let entry = entry.map_err(|error| fail("inspect parent entry", error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".rhost-write-") {
            return Err(format!("{context} left a helper temp: {name}"));
        }
    }
    Ok(())
}

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
