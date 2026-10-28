//! Native parser and byte-boundary conformance, independent of CLI formatting.
use crate::remote_fs_support::{ask, ask_raw, create, read_bytes, sha256_hex};
use crate::support::Harness;
use serde_json::json;

#[test]
fn malformed_json_never_reaches_a_named_file() -> Result<(), String> {
    let harness = Harness::open("native-file-parser")?;
    let path = create(&harness, "target", b"original\n")?;
    let encoded = serde_json::to_string(&path).map_err(|e| e.to_string())?;
    for suffix in [
        r#", "max_bytes": 4096, "max_bytes": 1}"#,
        r#", "max_bytes": 4096, "surprise": true}"#,
        r#", "max_bytes": true}"#,
        r#", "max_bytes": 1.0}"#,
        r#", "max_bytes": -1}"#,
        r#", "max_bytes": 18446744073709551616}"#,
        r#", "max_bytes": 4096} {}"#,
        r#", "max_bytes": 4096, "x": "\ud800"}"#,
        r#", "max_bytes": 4096, "\u006f\u0070": "write"}"#,
    ] {
        let request = format!("{{\"op\":\"read\",\"path\":{encoded}{suffix}");
        let result = ask_raw(&harness, request.as_bytes())?;
        assert_eq!(result["error"], "CONFIG_INVALID", "{request}: {result}");
        assert_eq!(read_bytes(&path)?, b"original\n");
    }
    let mut invalid = b"{\"op\":\"read\",\"path\":\"".to_vec();
    invalid.extend([0xc0, 0x80]);
    invalid.extend(b"\",\"max_bytes\":4096}");
    assert_eq!(ask_raw(&harness, &invalid)?["error"], "CONFIG_INVALID");
    let nul_path = format!("{path}\0ignored");
    assert_eq!(
        ask(
            &harness,
            json!({"op":"read","path":nul_path,"max_bytes":4096})
        )?["error"],
        "CONFIG_INVALID"
    );
    Ok(())
}

#[test]
fn nul_and_multibyte_text_survive_exact_patch_and_json_escaping() -> Result<(), String> {
    let harness = Harness::open("native-file-text")?;
    let original = "前\0后\nlast".as_bytes();
    let path = create(&harness, "text", original)?;
    let result = ask(&harness, json!({"op":"read","path":path,"max_bytes":4096}))?;
    assert_eq!(result["content"], "前\0后\nlast");
    assert_eq!(result["sha256"], sha256_hex(original));
    let result = ask(
        &harness,
        json!({"op":"patch","path":path,"max_bytes":4096,"if_hash":sha256_hex(original),"edits":[{"old":"前\0后","new":"替\0换"}]}),
    )?;
    assert_eq!(result["sha256"], sha256_hex("替\0换\nlast".as_bytes()));
    assert_eq!(read_bytes(&path)?, "替\0换\nlast".as_bytes());
    Ok(())
}

#[test]
fn an_oversized_line_is_omitted_but_still_contributes_to_file_identity() -> Result<(), String> {
    let harness = Harness::open("native-file-long-line")?;
    let mut original = vec![b'x'; 8 * 1024 * 1024 + 17];
    original.extend(b"\nlast\n");
    let path = create(&harness, "text", &original)?;
    let first = ask(&harness, json!({"op":"read","path":path,"max_bytes":4096}))?;
    assert_eq!(first["content"], "");
    assert_eq!(first["truncated"], true);
    assert_eq!(first["total_lines"], 2);
    assert_eq!(first["sha256"], sha256_hex(&original));
    let second = ask(
        &harness,
        json!({"op":"read","path":path,"start":2,"lines":1,"max_bytes":4096}),
    )?;
    assert_eq!(second["content"], "last\n");
    assert_eq!(second["sha256"], first["sha256"]);
    Ok(())
}

#[test]
fn sync_resolution_preserves_missing_trailing_directories() -> Result<(), String> {
    let harness = Harness::open("native-file-missing-resolution")?;
    let path = harness.path("not-created/nested/destination");
    let result = ask(&harness, json!({"op":"resolve","path":path,"delete":true}))?;
    assert_eq!(result["path"], path.to_string_lossy().as_ref());
    assert!(!path.exists(), "resolution is read-only");
    Ok(())
}
