//! The local audit trail: what a remote operation left behind, and what a later
//! process can read back.
//!
//! Logging is fail-open, so these cases check both halves: an entry appears
//! without the caller asking for one, and a trail that cannot be written never
//! changes the answer to the operation itself.
use crate::support::{Harness, envelope, fail, run, want_code};

/// A stub that makes any submission fail before it runs. That is enough to prove
/// the trail records what happened, without a stub fabricating completion
/// evidence (README.md, tests/acceptance/main.rs).
const DENIED: &str = "printf 'Permission denied (publickey).\\n' >&2\nexit 255";

fn entry_count(value: &serde_json::Value) -> usize {
    value
        .pointer("/data/entries")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or_default()
}

#[test]
fn a_remote_operation_is_recorded_and_read_back_by_a_later_process() -> Result<(), String> {
    let harness = Harness::open("audit-write")?;
    harness.stub("ssh", DENIED)?;
    let (_, failed) = envelope(
        &harness,
        &["exec", "gpu", "--json", "--command", "echo audited"],
    )?;
    want_code(&failed, "SSH_AUTH_FAILED")?;

    let (outcome, listed) = envelope(&harness, &["audit", "--json"])?;
    assert_eq!(outcome.status, 0, "{listed}");
    assert_eq!(
        listed["data"]["path"].as_str().map(str::to_string),
        Some(
            harness
                .path("state/v4/audit.jsonl")
                .to_string_lossy()
                .into_owned()
        )
    );
    let entries = listed["data"]["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(entries.len(), 1, "{listed}");
    let entry = &entries[0];
    assert_eq!(entry["operation"], "exec");
    assert_eq!(entry["host"], "gpu");
    assert_eq!(entry["ok"], serde_json::Value::Bool(false));
    assert_eq!(entry["error_code"], "SSH_AUTH_FAILED");
    assert!(
        entry["duration_ms"].as_u64().is_some(),
        "an operation without a duration is not evidence: {entry}"
    );
    assert_eq!(
        entry["time"].as_str().map(str::len),
        Some(20),
        "the trail publishes UTC RFC 3339: {entry}"
    );
    // The human view carries the same facts (AGENTS.md §6).
    let human = run(&harness, &["audit"])?;
    assert_eq!(human.status, 0);
    assert!(
        human.stdout.contains("SSH_AUTH_FAILED"),
        "{:?}",
        human.stdout
    );
    assert!(human.stdout.contains("exec"), "{:?}", human.stdout);
    Ok(())
}

#[test]
fn auditing_can_be_turned_off_and_then_records_nothing() -> Result<(), String> {
    let harness = Harness::open("audit-off")?.env("RHOST_AUDIT", "0");
    harness.stub("ssh", DENIED)?;
    envelope(&harness, &["exec", "gpu", "--json", "--command", "true"])?;
    let (_, listed) = envelope(&harness, &["audit", "--json"])?;
    assert_eq!(entry_count(&listed), 0, "{listed}");
    assert!(
        !harness.path("state/v4/audit.jsonl").exists(),
        "a disabled audit trail is not a file"
    );
    Ok(())
}

#[test]
fn the_audit_view_filters_by_host_and_keeps_the_tail() -> Result<(), String> {
    let harness = Harness::open("audit-view")?;
    harness.stub("ssh", DENIED)?;
    for host in ["gpu", "other", "gpu"] {
        envelope(&harness, &["exec", host, "--json", "--command", "true"])?;
    }
    let (_, all) = envelope(&harness, &["audit", "--json"])?;
    assert_eq!(entry_count(&all), 3, "{all}");

    let (_, only_gpu) = envelope(&harness, &["audit", "--json", "--host", "gpu"])?;
    assert_eq!(entry_count(&only_gpu), 2, "{only_gpu}");
    assert!(
        only_gpu["data"]["entries"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .all(|entry| entry["host"] == "gpu")
    );

    let (_, last) = envelope(&harness, &["audit", "--json", "--limit", "1"])?;
    assert_eq!(entry_count(&last), 1, "{last}");
    let (_, none) = envelope(
        &harness,
        &["audit", "--json", "--limit", "0", "--host", "nowhere"],
    )?;
    assert_eq!(entry_count(&none), 0, "{none}");

    for args in [
        vec!["audit", "--json", "--limit", "nope"],
        vec!["audit", "--json", "--limit", "-3"],
        vec!["audit", "--json", "extra"],
        vec!["audit", "--json", "--nope"],
    ] {
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(outcome.status, 255, "{args:?}");
        want_code(&value, "USAGE_ERROR").map_err(|error| format!("{args:?} {error}"))?;
    }
    Ok(())
}

#[test]
fn a_corrupt_line_is_skipped_and_a_trail_that_cannot_be_written_stays_fail_open()
-> Result<(), String> {
    let harness = Harness::open("audit-robust")?;
    harness.stub("ssh", DENIED)?;
    envelope(&harness, &["exec", "gpu", "--json", "--command", "true"])?;
    // Another process appended something unreadable: the rest of the trail is
    // still the trail.
    harness.write("state/v4/audit.jsonl", "{\"broken\":\n", None)?;
    let (outcome, listed) = envelope(&harness, &["audit", "--json"])?;
    assert_eq!(outcome.status, 0, "{listed}");
    assert_eq!(entry_count(&listed), 0, "{listed}");

    // A trail that cannot be written must not turn a completed operation into a
    // failure: the log is a record of work, not a precondition for it.
    std::fs::remove_file(harness.path("state/v4/audit.jsonl"))
        .map_err(|error| format!("remove trail: {error}"))?;
    std::fs::create_dir(harness.path("state/v4/audit.jsonl"))
        .map_err(|error| format!("replace trail with a directory: {error}"))?;
    let (outcome, value) = envelope(&harness, &["exec", "gpu", "--json", "--command", "true"])?;
    assert_eq!(outcome.status, 255, "{value}");
    want_code(&value, "SSH_AUTH_FAILED")?;
    assert!(
        outcome.stderr.contains("rhost: audit:"),
        "a write that did not land is reported: {:?}",
        outcome.stderr
    );
    Ok(())
}

#[test]
fn invalid_utf8_does_not_hide_valid_audit_entries_or_change_the_filtered_tail() -> Result<(), String>
{
    let harness = Harness::open("audit-invalid-utf8")?;
    let path = harness.path("state/v4/audit.jsonl");
    let parent = path.parent().ok_or("audit path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| fail("audit directory", error))?;
    let make = |host: &str, operation: &str| {
        format!(
            "{{\"time\":\"now\",\"host\":\"{host}\",\"operation\":\"{operation}\",\"duration_ms\":1,\"ok\":true}}\n"
        )
    };
    let mut body = make("gpu", "first").into_bytes();
    body.extend_from_slice(b"\xff\n");
    body.extend_from_slice(make("other", "middle").as_bytes());
    body.extend_from_slice(make("gpu", "last").as_bytes());
    std::fs::write(&path, body).map_err(|error| fail("audit fixture", error))?;
    let (outcome, value) = envelope(
        &harness,
        &["audit", "--json", "--host", "gpu", "--limit", "1"],
    )?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(entry_count(&value), 1, "{value}");
    assert_eq!(value["data"]["entries"][0]["operation"], "last");
    let (_, all) = envelope(&harness, &["audit", "--json", "--limit", "0"])?;
    assert_eq!(entry_count(&all), 3, "{all}");
    Ok(())
}

#[test]
fn audit_never_records_environment_values() -> Result<(), String> {
    let harness = Harness::open("audit-redaction")?;
    harness.stub("ssh", DENIED)?;
    let secret = "fixture-sensitive-value";
    let _ = envelope(
        &harness,
        &[
            "exec",
            "gpu",
            "--json",
            "--env",
            &format!("RHOST_FIXTURE_SECRET={secret}"),
            "--command",
            "true",
        ],
    )?;
    let (_, value) = envelope(&harness, &["audit", "--json"])?;
    let encoded = serde_json::to_string(&value).map_err(|error| error.to_string())?;
    assert!(
        !encoded.contains(secret),
        "audit response leaked an environment value"
    );
    let raw = std::fs::read_to_string(harness.path("state/v4/audit.jsonl"))
        .map_err(|error| format!("read audit: {error}"))?;
    assert!(
        !raw.contains(secret),
        "audit file leaked an environment value"
    );
    Ok(())
}
