use crate::support::{Harness, envelope_with_stdin};
use serde_json::json;

#[test]
fn structured_files_share_cas_and_do_not_require_shell_quoting() -> Result<(), String> {
    let harness = Harness::open("request-file")?;
    crate::hosts::local_host(&harness)?;
    let path = harness
        .path("file 'quotes'; dollar$()")
        .to_string_lossy()
        .into_owned();
    let request = |operation: &str, params| {
        json!({"schema_version":3,"host":"gpu","operation":operation,"params":params}).to_string()
    };
    let (outcome, written) = envelope_with_stdin(
        &harness,
        &["request", "--from", "-"],
        request("fs.write", json!({"path":path,"content":"before\n"})).as_bytes(),
    )?;
    assert_eq!(outcome.status, 0, "{written}");
    assert_eq!(written["data"]["mutation"], "applied");
    let hash = written["data"]["sha256"].clone();
    let (outcome, patched) = envelope_with_stdin(
        &harness,
        &["request", "--from", "-"],
        request(
            "fs.patch",
            json!({"path":path,"if_hash":hash,"edits":[{"old":"before","new":"after"}]}),
        )
        .as_bytes(),
    )?;
    assert_eq!(outcome.status, 0, "{patched}");
    assert_eq!(patched["data"]["mutation"], "applied");
    let (_, read) = envelope_with_stdin(
        &harness,
        &["request", "--from", "-"],
        request("fs.read", json!({"path":path})).as_bytes(),
    )?;
    assert_eq!(read["data"]["content"], "after\n");
    let (_, stale) = envelope_with_stdin(
        &harness,
        &["request", "--from", "-"],
        request(
            "fs.write",
            json!({"path":path,"content":"stale\n","if_hash":hash}),
        )
        .as_bytes(),
    )?;
    assert_eq!(stale["error"]["code"], "FILE_CONFLICT");
    assert_eq!(stale["error"]["mutation"], "not_applied");
    Ok(())
}

#[test]
fn raw_request_duplicates_and_invalid_file_parameters_stop_before_ssh() -> Result<(), String> {
    for (index, raw) in [
        r#"{"schema_version":3,"operation":"fs.write","host":"gpu","params":{"path":"/srv/file","content":"a","content":"b"}}"#,
        r#"{"schema_version":3,"operation":"fs.write","host":"gpu","params":{"path":"/srv/file","content":true}}"#,
        r#"{"schema_version":3,"operation":"fs.read","host":"gpu","params":{"path":"/srv/file","max_bytes":0}}"#,
        r#"{"schema_version":3,"operation":"fs.patch","host":"gpu","params":{"path":"/srv/file","if_hash":"","edits":[]}}"#,
        r#"{"schema_version":3,"operation":"run.list","host":"gpu","host":"other","params":{}}"#,
        r#"{"schema_version":3,"operation":"metadata","host":"gpu","if_boot_id":"invalid","params":{}}"#,
        r#"{"schema_version":3,"operation":"metadata","host":"gpu","if_boot_id":null,"params":{}}"#,
    ].into_iter().enumerate() {
        let harness = Harness::open(&format!("request-invalid-{index}"))?;
        let (outcome, answer) = envelope_with_stdin(&harness, &["request","--from","-"],raw.as_bytes())?;
        assert_eq!(outcome.status,255,"{answer}");
        assert_eq!(answer["error"]["code"],"CONFIG_INVALID");
        harness.assert_no_remote_tool("invalid request")?;
    }
    Ok(())
}

#[test]
fn changeset_failure_keeps_its_record_in_the_public_envelope() -> Result<(), String> {
    let harness = Harness::open("request-changeset")?;
    crate::hosts::local_host(&harness)?;
    let state = harness.path("remote-state").to_string_lossy().into_owned();
    let harness = harness.env("RHOST_REMOTE_STATE", &state);
    let call = |op: &str, params: serde_json::Value| {
        envelope_with_stdin(
            &harness,
            &["request", "--from", "-"],
            json!({"schema_version":3,"operation":op,"host":"gpu","params":params})
                .to_string()
                .as_bytes(),
        )
    };
    let (_, prepared) = call("helper.prepare", json!({}))?;
    let generation = prepared["data"]["generation"].clone();
    let expiry = prepared["data"]["remote_time_unix"]
        .as_u64()
        .ok_or("time")?
        + 3600;
    let a = harness.path("a").to_string_lossy().into_owned();
    let b = harness.path("b").to_string_lossy().into_owned();
    std::fs::write(&b, b"external").map_err(|e| e.to_string())?;
    let (outcome, result) = call(
        "fs.changeset.apply",
        json!({"generation":generation,"key":format!("{expiry}-{}","3".repeat(64)),"expires_unix":expiry,"changes":[{"op":"write","path":a,"content":"new"},{"op":"write","path":b,"content":"conflict"}]}),
    )?;
    assert_eq!(outcome.status, 255);
    assert_eq!(result["error"]["code"], "HASH_REQUIRED", "{result}");
    assert_eq!(result["data"]["mutation"], "not_applied");
    assert_eq!(result["data"]["files"].as_array().ok_or("files")?.len(), 2);
    assert!(!std::path::Path::new(&a).exists());
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../schemas/result-v3.schema.json"))
            .map_err(|e| e.to_string())?;
    let validator = jsonschema::validator_for(&schema).map_err(|e| e.to_string())?;
    assert!(validator.is_valid(&result), "{result}");
    Ok(())
}
