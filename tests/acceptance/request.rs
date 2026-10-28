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
