use super::support::{Live, text, unique};

#[test]
fn audit_survives_processes_and_redacts_payloads() -> Result<(), String> {
    let live = Live::new("audit")?;
    let secret = unique("private-payload");
    let command = "printf '%s' \"$RHOST_AUDIT_SECRET\"".to_string();
    live.ok(&[
        "--json",
        "exec",
        live.host(),
        "--env",
        &format!("RHOST_AUDIT_SECRET={secret}"),
        "--command",
        &command,
    ])?;
    let audit = live.ok(&["--json", "audit", "--limit", "1"])?;
    let encoded = serde_json::to_string(&audit.value)
        .map_err(|error| format!("encode audit answer: {error}"))?;
    assert!(
        !encoded.contains(&secret),
        "audit leaked an environment value"
    );
    assert_eq!(text(&audit.value, "/data/entries/0/operation")?, "exec");
    assert_eq!(text(&audit.value, "/data/entries/0/host")?, live.host());
    Ok(())
}

#[test]
fn concurrent_audit_appends_leave_valid_json_lines() -> Result<(), String> {
    let live = Live::new("audit-concurrent")?;
    let mut children = Vec::new();
    for index in 0..8 {
        let command = format!("printf audit-{index}");
        children.push(
            live.command(&["--json", "exec", live.host(), "--command", &command])
                .spawn()
                .map_err(|error| format!("spawn audited command: {error}"))?,
        );
    }
    for child in children {
        let output = super::support::wait_bounded(child, std::time::Duration::from_secs(90))?;
        let answer = super::support::decode(output, &["audited command"])?;
        assert_eq!(answer.value["ok"], true);
    }
    let audit = live.ok(&["--json", "audit", "--limit", "8"])?;
    assert_eq!(
        audit.value["data"]["entries"].as_array().map(Vec::len),
        Some(8)
    );
    // The public reader parsing all eight records is the cross-process integrity
    // assertion; no test reaches into the implementation's state file format.
    Ok(())
}
