//! Core operations share their capability results across all structured entries.
use crate::mcp::Mcp;
use crate::support::{Harness, envelope, envelope_with_stdin, want_code};
use serde_json::{Value, json};

fn request(h: &Harness, op: &str, host: Option<&str>, params: Value) -> Result<Value, String> {
    let mut raw = json!({"schema_version":3,"operation":op,"params":params});
    if let Some(host) = host {
        raw["host"] = host.into();
    }
    let (_, result) =
        envelope_with_stdin(h, &["request", "--from", "-"], raw.to_string().as_bytes())?;
    Ok(result)
}

#[test]
fn local_discovery_has_no_target_and_matches_cli_and_mcp() -> Result<(), String> {
    let h = Harness::open("core-discovery")?;
    h.write(
        "home/.ssh/config",
        "Host gpu edge\n  HostName example-host\n",
        None,
    )?;
    let mut m = Mcp::open(&h)?;
    for op in ["version", "hosts"] {
        let (_, cli) = envelope(&h, &[op, "--json"])?;
        let document = request(&h, op, None, json!({}))?;
        let tool = m.call(3, &format!("rhost_{op}"), json!({"params":{}}))?;
        assert_eq!(document, cli);
        assert_eq!(tool, cli);
    }
    m.close()?;
    h.assert_no_remote_tool("local discovery")
}

#[test]
fn core_invalid_parameters_and_target_policies_refuse_before_ssh() -> Result<(), String> {
    let cases = [
        ("version", json!({"extra":true})),
        ("hosts", json!({"extra":true})),
        ("doctor", json!({"timeout_ms":-1})),
        ("connection.status", json!({"fresh":true})),
        ("session.create", json!({"shell":"sh"})),
        ("session.list", json!({"timeout_ms":null})),
        ("session.exec", json!({"session":"debug","command":" \t"})),
        (
            "session.send",
            json!({"session":"debug","data":"x","key":"C-c"}),
        ),
        (
            "session.read",
            json!({"session":"debug","since":0,"tail":4}),
        ),
        ("session.recover", json!({"session":""})),
        ("session.close", json!({"session":"debug","timeout_ms":2})),
    ];
    for (i, (op, params)) in cases.into_iter().enumerate() {
        let h = Harness::open(&format!("core-invalid-{i}"))?;
        let local = matches!(op, "version" | "hosts");
        let result = request(
            &h,
            op,
            if local { None } else { Some("gpu") },
            params.clone(),
        )?;
        want_code(&result, "CONFIG_INVALID")?;
        let mut m = Mcp::open(&h)?;
        let mut args = json!({"params":params});
        if !local {
            args["host"] = "gpu".into();
        }
        let result = m.call(3, &format!("rhost_{}", op.replace('.', "_")), args)?;
        want_code(&result, "CONFIG_INVALID")?;
        m.close()?;
        h.assert_no_remote_tool("invalid core parameters")?;
    }
    for (i, raw) in [
        json!({"schema_version":3,"operation":"version","host":null,"params":{}}),
        json!({"schema_version":3,"operation":"hosts","host":"gpu","params":{}}),
        json!({"schema_version":3,"operation":"hosts","if_boot_id":null,"params":{}}),
        json!({"schema_version":3,"operation":"session.list","params":{}}),
        json!({"schema_version":3,"operation":"session.list","host":"gpu","if_boot_id":"00000000-0000-0000-0000-000000000000","params":{}}),
    ].into_iter().enumerate() {
        let h = Harness::open(&format!("core-target-{i}"))?;
        let (_, result) = envelope_with_stdin(&h, &["request","--from","-"], raw.to_string().as_bytes())?;
        want_code(&result, "CONFIG_INVALID")?;
        h.assert_no_remote_tool("invalid target policy")?;
    }
    Ok(())
}

#[test]
fn diagnostic_failure_keeps_the_same_report_and_control_evidence() -> Result<(), String> {
    let h = Harness::open("core-diagnostic")?;
    h.stub(
        "ssh",
        "printf 'Permission denied (publickey).\\n' >&2\nexit 255",
    )?;
    let mut m = Mcp::open(&h)?;
    for (op, argv) in [
        ("doctor", vec!["doctor", "gpu", "--json"]),
        (
            "connection.status",
            vec!["connection", "status", "gpu", "--json"],
        ),
    ] {
        let (_, cli) = envelope(&h, &argv)?;
        let result = request(&h, op, Some("gpu"), json!({}))?;
        let tool = m.call(
            3,
            &format!("rhost_{}", op.replace('.', "_")),
            json!({"host":"gpu","params":{}}),
        )?;
        assert_eq!(result, cli);
        assert_eq!(tool, cli);
        assert!(cli["data"].is_object());
    }
    m.close()
}

#[test]
fn session_refusals_reuse_existing_results_without_preparing_native_helper() -> Result<(), String> {
    let h = Harness::open("core-session-refusal")?;
    crate::hosts::local_host(&h)?;
    let state = h.path("remote-state").to_string_lossy().into_owned();
    let h = h
        .env("RHOST_REMOTE_STATE", &state)
        .env("RHOST_NO_UPLOAD", "1");
    let mut m = Mcp::open(&h)?;
    for (op, params, args) in [
        (
            "session.exec",
            json!({"session":"absent","command":"printf unexpected"}),
            vec![
                "session",
                "exec",
                "gpu",
                "absent",
                "--command",
                "printf unexpected",
                "--json",
            ],
        ),
        (
            "session.read",
            json!({"session":"absent","max_bytes":4}),
            vec![
                "session",
                "read",
                "gpu",
                "absent",
                "--max-bytes",
                "4",
                "--json",
            ],
        ),
        (
            "session.close",
            json!({"session":"absent"}),
            vec!["session", "close", "gpu", "absent", "--json"],
        ),
    ] {
        let (_, cli) = envelope(&h, &args)?;
        let result = request(&h, op, Some("gpu"), params.clone())?;
        let tool = m.call(
            3,
            &format!("rhost_{}", op.replace('.', "_")),
            json!({"host":"gpu","params":params}),
        )?;
        assert_eq!(result, cli);
        assert_eq!(tool, cli);
        want_code(&tool, "SESSION_NOT_FOUND")?;
        assert_eq!(tool["error"]["retryable"], false);
    }
    m.close()
}

#[test]
fn positional_json_shapes_never_submit_core_operations() -> Result<(), String> {
    let h = Harness::open("core-object-shapes")?;
    for raw in [
        r#"[3,"session.close","gpu",{"session":"debug"}]"#,
        r#"{"schema_version":3,"operation":"version","params":[]}"#,
        r#"{"schema_version":3,"operation":"session.close","host":"gpu","params":["debug"]}"#,
    ] {
        let (_, result) = envelope_with_stdin(&h, &["request", "--from", "-"], raw.as_bytes())?;
        want_code(&result, "CONFIG_INVALID")?;
    }
    let mut m = Mcp::open(&h)?;
    for (op, params) in [
        ("version", json!([])),
        ("hosts", json!(null)),
        ("doctor", json!([0, false])),
        ("session.create", json!([])),
        ("session.send", json!(["debug", "input"])),
        ("session.close", json!(["debug"])),
    ] {
        let mut args = json!({"params":params});
        if !matches!(op, "version" | "hosts") {
            args["host"] = "gpu".into();
        }
        let result = m.call(3, &format!("rhost_{}", op.replace('.', "_")), args)?;
        want_code(&result, "CONFIG_INVALID")?;
    }
    m.close()?;
    h.assert_no_remote_tool("positional documents and parameters")
}

#[test]
fn an_empty_cli_session_shell_keeps_the_bash_default() -> Result<(), String> {
    let h = Harness::open("core-cli-empty-shell")?;
    h.stub(
        "ssh",
        "printf 'Permission denied (publickey).\\n' >&2\nexit 255",
    )?;
    for shell in ["bash", ""] {
        let (_, result) = envelope(
            &h,
            &["session", "create", "gpu", "--shell", shell, "--json"],
        )?;
        want_code(&result, "SSH_AUTH_FAILED")?;
    }
    assert_eq!(h.calls().len(), 2);
    Ok(())
}
