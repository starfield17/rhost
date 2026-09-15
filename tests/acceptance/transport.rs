//! Connection reuse, control-protocol status and the doctor probe.
//!
//! A stub ssh answers exactly the control exchanges a real one would, so these
//! cases pin what rhost reports about a master without owning one.
use crate::hosts::local_host;
use crate::support::{Harness, envelope, operation, run, shell_quote, want_code};

/// A stub OpenSSH that answers the control protocol and nothing else. The mode
/// comes from the environment, so one script covers an alive, an absent and an
/// unreadable master without three fixtures.
const MASTER_SCRIPT: &str = r#"case "$RHOST_TEST_MASTER_MODE" in
  absent) echo 'Control socket connect(/tmp/rhost-test): No such file or directory' >&2; exit 255 ;;
  unknown) echo 'Control socket connect(/tmp/rhost-test): Permission denied' >&2; exit 255 ;;
esac
case "$*" in
  *'-O check'*) echo 'Master running (pid=4321)' >&2; exit 0 ;;
  *'-O stop'*) echo 'Stop listening request sent.' >&2; exit 0 ;;
esac
exit 2
"#;

#[test]
fn connection_status_and_reset_drive_the_control_protocol_only() -> Result<(), String> {
    let harness = Harness::open("connection")?;
    harness.stub("ssh", MASTER_SCRIPT)?;
    let harness = harness.env("RHOST_TEST_MASTER_MODE", "alive");
    let (outcome, value) = envelope(&harness, &["connection", "status", "gpu", "--json"])?;
    assert_eq!(outcome.status, 0);
    assert_eq!(operation(&value), "connection.status");
    assert_eq!(
        value["data"]["master_status"],
        serde_json::Value::String("alive".into())
    );
    assert_eq!(
        value["data"]["master_pid"],
        serde_json::Value::from(4321),
        "the pid OpenSSH named must be carried, not invented"
    );
    assert!(
        !harness.calls_text().contains("-O exit"),
        "reset must never terminate accepted channels: {}",
        harness.calls_text()
    );
    let (outcome, value) = envelope(&harness, &["connection", "reset", "gpu", "--json"])?;
    assert_eq!(outcome.status, 0);
    assert_eq!(operation(&value), "connection.reset");
    assert_eq!(value["data"]["stopped"], serde_json::Value::Bool(true));
    assert_eq!(
        value["data"]["master_status"],
        serde_json::Value::String("alive".into())
    );
    let calls = harness.calls_text();
    assert!(calls.contains("-O stop"), "reset must use stop: {calls}");
    assert!(
        !calls.contains("-O exit"),
        "reset must never use exit: {calls}"
    );
    // The human rendering names the same master state (§6).
    let human = run(&harness, &["connection", "status", "gpu"])?;
    assert_eq!(human.status, 0);
    assert!(human.stdout.contains("alive"), "{:?}", human.stdout);
    assert!(human.stdout.contains("4321"), "{:?}", human.stdout);
    Ok(())
}

#[test]
fn an_absent_master_is_reported_without_error_and_an_unknown_one_fails_reset() -> Result<(), String>
{
    for mode in ["absent", "unknown"] {
        let harness = Harness::open(&format!("connection-{mode}"))?;
        harness.stub("ssh", MASTER_SCRIPT)?;
        let harness = harness.env("RHOST_TEST_MASTER_MODE", mode);
        let (outcome, value) = envelope(&harness, &["connection", "status", "gpu", "--json"])?;
        assert_eq!(outcome.status, 0, "a status read is never an error");
        assert_eq!(
            value["data"]["master_status"],
            serde_json::Value::String(mode.into()),
            "{mode}"
        );
        assert!(
            value["data"].get("master_pid").is_none(),
            "no pid may be invented for {mode}: {}",
            value["data"]
        );
        let (outcome, value) = envelope(&harness, &["connection", "reset", "gpu", "--json"])?;
        if mode == "absent" {
            assert_eq!(outcome.status, 0, "nothing to stop is not a failure");
            assert_eq!(value["ok"], serde_json::Value::Bool(true));
            assert_eq!(
                value["data"].get("stopped"),
                None,
                "an absent master reports no stop"
            );
        } else {
            assert_eq!(outcome.status, 255);
            want_code(&value, "SSH_CONTROL_FAILED")?;
            assert_eq!(value["error"]["retryable"], serde_json::Value::Bool(false));
        }
    }
    Ok(())
}

#[test]
fn fresh_mode_never_initializes_shared_state() -> Result<(), String> {
    let harness = Harness::open("fresh")?;
    harness.stub("ssh", "exit 255")?;
    let (_, value) = envelope(
        &harness,
        &["exec", "gpu", "--json", "--fresh", "--command", "date"],
    )?;
    want_code(&value, "REMOTE_EXECUTION_UNKNOWN")?;
    let calls = harness.calls();
    let submission = calls
        .iter()
        .find(|call| call.args.iter().any(|argument| argument == "-T"))
        .ok_or("the stub recorded no submission")?;
    assert!(
        submission
            .args
            .iter()
            .any(|argument| argument == "ControlMaster=no")
            && submission
                .args
                .iter()
                .any(|argument| argument == "ControlPath=none"),
        "a fresh run must not touch shared state: {submission:?}"
    );
    assert!(
        !submission.args.iter().any(|argument| argument == "-O"),
        "a fresh run has no master to query: {submission:?}"
    );
    assert_eq!(
        submission
            .args
            .iter()
            .filter(|argument| **argument == "-T")
            .count(),
        1,
        "exactly one -T, and never a pseudo-terminal: {submission:?}"
    );
    // Exactly one shell program crosses the boundary: the target and one command
    // string follow `--`, and nothing else (EXEC-001).
    let boundary = submission
        .args
        .iter()
        .position(|argument| argument == "--")
        .ok_or("no -- boundary in the submission")?;
    assert_eq!(
        submission.args.get(boundary + 1).map(String::as_str),
        Some("gpu")
    );
    assert_eq!(
        submission.args.len() - boundary - 1,
        2,
        "target plus one shell program, not a rebuilt argv: {submission:?}"
    );
    assert!(
        !submission.args[..boundary]
            .iter()
            .any(|argument| argument == "gpu"),
        "the target appears once, after the boundary"
    );
    Ok(())
}

#[test]
fn a_failed_probe_reports_the_connection_it_knew_and_no_capabilities() -> Result<(), String> {
    let harness = Harness::open("doctor")?;
    harness.stub(
        "ssh",
        "case \"$*\" in\n  *\"-O check\"*) exit 255 ;;\nesac\nprintf 'Permission denied (publickey).\\n' >&2\nexit 255\n",
    )?;
    let (outcome, value) = envelope(&harness, &["doctor", "gpu", "--json"])?;
    assert_eq!(outcome.status, 255);
    assert_eq!(operation(&value), "doctor");
    want_code(&value, "SSH_AUTH_FAILED")?;
    assert_eq!(value["data"]["online"], serde_json::Value::Bool(false));
    assert_eq!(
        value["data"]["capabilities"],
        serde_json::Value::Object(serde_json::Map::new()),
        "a probe that never completed has no capability answers"
    );
    assert_eq!(
        value["data"]["capability_paths"],
        serde_json::Value::Object(serde_json::Map::new()),
        "a probe that never completed has no resolved paths either"
    );
    assert_eq!(
        value["data"]["connection"]["master_status"],
        serde_json::Value::String("unknown".into()),
        "the master state read before the probe is still evidence"
    );
    assert_eq!(
        value["data"].get("connection_reused"),
        Some(&serde_json::Value::Null),
        "reuse could not be told"
    );
    let human = run(&harness, &["doctor", "gpu"])?;
    assert_eq!(human.status, 255);
    assert!(
        human.stderr.contains("SSH_AUTH_FAILED"),
        "{:?}",
        human.stderr
    );
    Ok(())
}

/// A completed probe reports the path this execution environment resolves each
/// capability to, under the same key set as `capabilities`, and the human view
/// names it. A host stand-in runs the probe exactly as a real one would.
#[test]
fn a_completed_probe_reports_the_path_each_capability_resolved_to() -> Result<(), String> {
    let harness = Harness::open("doctor-paths")?;
    local_host(&harness)?;
    let (outcome, value) = envelope(&harness, &["doctor", "gpu", "--json"])?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(value["data"]["online"], serde_json::Value::Bool(true));

    let capabilities = value["data"]["capabilities"]
        .as_object()
        .ok_or("capabilities is not an object")?;
    let paths = value["data"]["capability_paths"]
        .as_object()
        .ok_or("capability_paths is not an object")?;
    let mut capability_keys: Vec<&String> = capabilities.keys().collect();
    let mut path_keys: Vec<&String> = paths.keys().collect();
    capability_keys.sort();
    path_keys.sort();
    assert_eq!(
        capability_keys, path_keys,
        "both maps must publish the same key set"
    );

    // A capability this host has must carry the path `command -v` returned; a
    // capability it lacks must carry null, never a fabricated path.
    let mut seen_found = false;
    for (name, found) in capabilities {
        let found = found.as_bool().ok_or("capability is not a boolean")?;
        let path = paths
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if found {
            seen_found = true;
            assert!(
                path.as_deref().is_some_and(|value| value.starts_with('/')),
                "{name} is present but has no resolved path: {value}"
            );
        } else {
            assert_eq!(path, None, "{name} is missing but names a path");
        }
    }
    assert!(
        seen_found,
        "the probe needs at least one present capability to be a useful test"
    );

    // The human line carries the same fact the envelope does.
    let human = run(&harness, &["doctor", "gpu"])?;
    assert_eq!(human.status, 0);
    assert!(
        human.stdout.contains("OK (/"),
        "the human view must name the resolved path: {:?}",
        human.stdout
    );
    Ok(())
}

#[test]
fn a_fresh_probe_says_it_reused_nothing() -> Result<(), String> {
    let harness = Harness::open("doctor-fresh")?;
    harness.stub("ssh", "exit 255")?;
    let (_, value) = envelope(&harness, &["doctor", "gpu", "--json", "--fresh"])?;
    assert_eq!(
        value["data"]["connection_reused"],
        serde_json::Value::Bool(false),
        "fresh never rides shared state (SSH-002)"
    );
    assert!(
        harness.calls().into_iter().any(|call| {
            call.args.iter().any(|argument| argument == "-T")
                && call
                    .args
                    .iter()
                    .any(|argument| argument == "ControlMaster=no")
                && call
                    .args
                    .iter()
                    .any(|argument| argument == "ControlPath=none")
        }),
        "the probe itself must ride no shared master"
    );
    Ok(())
}

#[test]
fn an_explicit_zero_doctor_timeout_uses_the_default_budget() -> Result<(), String> {
    let harness = Harness::open("doctor-zero-timeout")?;
    harness.stub(
        "ssh",
        "case \"$*\" in\n  *\"-O check\"*) exit 255 ;;\nesac\nsleep 1\nprintf 'Permission denied (publickey).\\n' >&2\nexit 255\n",
    )?;
    let (outcome, value) = envelope(&harness, &["doctor", "gpu", "--json", "--timeout", "0"])?;
    assert_eq!(outcome.status, 255);
    want_code(&value, "SSH_AUTH_FAILED")?;
    assert_ne!(
        value
            .pointer("/error/code")
            .and_then(serde_json::Value::as_str),
        Some("REMOTE_COMMAND_TIMEOUT"),
        "zero selects the default probe budget rather than an immediate deadline"
    );
    Ok(())
}

#[test]
fn transport_diagnostics_are_classified_without_fabricating_completion() -> Result<(), String> {
    for case in [
        ("silent-channel-loss", "", "REMOTE_EXECUTION_UNKNOWN", false),
        (
            "reset-after-submission",
            "Read from remote host example-host: Connection reset by peer",
            "REMOTE_EXECUTION_UNKNOWN",
            false,
        ),
        (
            "authentication",
            "Permission denied (publickey).",
            "SSH_AUTH_FAILED",
            false,
        ),
        (
            "host-key",
            "Host key verification failed.",
            "HOST_KEY_FAILED",
            false,
        ),
        (
            "connection-refused",
            "ssh: connect to host example-host port 22: Connection refused",
            "SSH_UNREACHABLE",
            true,
        ),
        (
            "unresolvable",
            "ssh: Could not resolve hostname example-host: nodename nor servname provided",
            "HOST_UNKNOWN",
            false,
        ),
        (
            "missing-dependency",
            "bash: line 1: setsid: command not found",
            "REMOTE_DEPENDENCY_MISSING",
            false,
        ),
    ] {
        let (name, diagnostic, want, retryable) = case;
        let harness = Harness::open(&format!("classify-{name}"))?;
        harness.stub(
            "ssh",
            &format!("printf '%s\\n' {} >&2\nexit 255", shell_quote(diagnostic)),
        )?;
        let (outcome, value) = envelope(
            &harness,
            &[
                "exec",
                "gpu",
                "--json",
                "--fresh",
                "--command",
                "printf side-effect",
            ],
        )?;
        assert_eq!(outcome.status, 255, "{name}");
        want_code(&value, want).map_err(|error| format!("{name} {error}"))?;
        assert_eq!(
            value["error"]["retryable"],
            serde_json::Value::Bool(retryable),
            "{name}: only a pre-submission connection failure may be retried"
        );
        let execution = value
            .pointer("/data/execution/status")
            .and_then(serde_json::Value::as_str);
        if matches!(
            want,
            "SSH_AUTH_FAILED"
                | "HOST_KEY_FAILED"
                | "SSH_UNREACHABLE"
                | "HOST_UNKNOWN"
                | "REMOTE_DEPENDENCY_MISSING"
        ) {
            assert_eq!(
                execution,
                Some("not_started"),
                "{name} failed before submission"
            );
            assert_eq!(
                value
                    .pointer("/data/cleanup/status")
                    .and_then(serde_json::Value::as_str),
                Some("not_attempted"),
                "{name}"
            );
        } else {
            assert_eq!(execution, Some("unknown"), "{name} may have run");
        }
    }
    Ok(())
}
