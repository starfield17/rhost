//! Foreground execution: one shell program, honest evidence, real statuses.
//!
//! Everything here runs the candidate the way an agent does, and asserts on the
//! envelope rather than on prose. The stubs stop at the OpenSSH boundary, so what
//! a case proves about completion evidence comes from the candidate's own
//! protocol, never from a fixture that parrots it.
use crate::hosts::local_host;
use crate::support::{Harness, SCHEMA, envelope, fail, want_code};

use std::process::{Command, Stdio};

#[test]
fn local_wrapper_smoke_preserves_real_completion_and_streams() -> Result<(), String> {
    let harness = Harness::open("exec-local-wrapper")?;
    local_host(&harness)?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "exec",
            "gpu",
            "--json",
            "--fresh",
            "--command",
            "printf local-out; printf local-err >&2; exit 7",
        ],
    )?;
    assert_eq!(outcome.status, 7, "{value}");
    assert_eq!(value["data"]["execution"]["status"], "completed");
    assert_eq!(value["data"]["execution"]["exit_code"], 7);
    assert_eq!(value["data"]["output"]["stdout"]["content"], "local-out");
    assert_eq!(value["data"]["output"]["stderr"]["content"], "local-err");
    assert_eq!(harness.tool_calls("ssh").len(), 1);
    Ok(())
}

/// A stub whose first answer hangs and whose second answer fails fast: the
/// cleanup call must not be the thing that decides the reported outcome.
fn hanging_stub(harness: &Harness) -> Result<(), String> {
    harness.stub(
        "ssh",
        "if [ -f \"$RHOST_TEST_SECOND\" ]; then exit 7; fi\n: > \"$RHOST_TEST_SECOND\"\nsleep 30\n",
    )
}

#[test]
fn an_explicit_deadline_bounds_the_local_invocation_and_is_not_retryable() -> Result<(), String> {
    let harness = Harness::open("timeout")?;
    hanging_stub(&harness)?;
    let second = harness.path("second");
    let harness = harness.env("RHOST_TEST_SECOND", second.to_str().ok_or("path")?);
    let started = std::time::Instant::now();
    let (outcome, value) = envelope(
        &harness,
        &[
            "exec",
            "gpu",
            "--json",
            "--fresh",
            "--timeout",
            "1s",
            "--command",
            "sleep 30",
        ],
    )?;
    assert!(
        started.elapsed() < std::time::Duration::from_secs(25),
        "the deadline must end the local wait"
    );
    assert_eq!(outcome.status, 124, "a deadline is 124, not 255 (WIRE-004)");
    want_code(&value, "REMOTE_COMMAND_TIMEOUT")?;
    assert_eq!(
        value["error"]["retryable"],
        serde_json::Value::Bool(false),
        "a timed-out command may already have acted (EXEC-006)"
    );
    let cleanup = value
        .pointer("/data/cleanup/status")
        .and_then(serde_json::Value::as_str);
    assert!(
        matches!(cleanup, Some("unconfirmed") | Some("confirmed_stopped")),
        "a timeout must say what cleanup proved: {value}"
    );
    assert_eq!(
        value
            .pointer("/data/execution/status")
            .and_then(serde_json::Value::as_str),
        Some("unknown"),
        "no completion evidence appeared, so no exit may be claimed"
    );
    Ok(())
}

#[test]
fn a_cancelled_run_reports_the_signal_that_ended_it() -> Result<(), String> {
    let harness = Harness::open("cancel")?;
    hanging_stub(&harness)?;
    let second = harness.path("second");
    let harness = harness.env("RHOST_TEST_SECOND", second.to_str().ok_or("path")?);
    let child = harness
        .command(&["exec", "gpu", "--json", "--fresh", "--command", "sleep 30"])?
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| fail("spawn candidate", error))?;
    let pid = child.id().to_string();
    // Wait until the stub has recorded its call, so the signal cannot arrive
    // before the submission it is meant to interrupt.
    let mut waited = std::time::Duration::ZERO;
    while !harness.path("calls").exists() && waited < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(20));
        waited += std::time::Duration::from_millis(20);
    }
    let status = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .map_err(|error| fail("signal candidate", error))?;
    assert!(status.success(), "the test could not signal the candidate");
    let output = child
        .wait_with_output()
        .map_err(|error| fail("collect candidate output", error))?;
    let body = String::from_utf8_lossy(&output.stdout).into_owned();
    let value: serde_json::Value = serde_json::from_str(body.trim())
        .map_err(|error| fail("one document", format!("{error} {body:?}")))?;
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).map_err(|error| fail("load schema", error))?;
    let validator =
        jsonschema::validator_for(&schema).map_err(|error| fail("compile schema", error))?;
    assert!(validator.is_valid(&value), "rejected envelope: {value}");
    assert_eq!(
        output.status.code().unwrap_or(-1),
        143,
        "SIGTERM cancellation is 143 (WIRE-004)"
    );
    want_code(&value, "REMOTE_COMMAND_CANCELLED")?;
    assert_eq!(
        value
            .pointer("/data/cancel_signal")
            .and_then(serde_json::Value::as_str),
        Some("SIGTERM"),
        "the reason must be observed, not guessed"
    );
    assert_eq!(value["error"]["retryable"], serde_json::Value::Bool(false));
    Ok(())
}

#[test]
fn a_lost_stdout_is_reported_on_stderr_and_never_resend() -> Result<(), String> {
    for args in [
        vec!["version", "--json"],
        vec!["hosts", "--json"],
        vec!["--json"],
    ] {
        let harness = Harness::open("lost-stdout")?;
        let mut child = harness
            .command(&args)?
            .stdout(Stdio::piped())
            .stderr(Stdio::from(
                std::fs::File::create(harness.path("stderr")).map_err(|e| fail("open sink", e))?,
            ))
            .spawn()
            .map_err(|error| fail("spawn candidate", error))?;
        // Closing the read end before the candidate writes turns stdout into a
        // broken pipe.
        drop(child.stdout.take());
        let status = child
            .wait()
            .map_err(|error| fail("wait for candidate", error))?;
        let stderr = std::fs::read_to_string(harness.path("stderr"))
            .map_err(|error| fail("read stderr", error))?;
        assert_eq!(
            status.code().unwrap_or(-1),
            255,
            "a lost document is not the operation's own status: {args:?}"
        );
        assert!(
            stderr.contains("OUTPUT_WRITE_FAILED"),
            "{args:?} lost stdout said: {stderr:?}"
        );
        assert!(
            stderr.contains("may have completed"),
            "{args:?} must not imply the operation failed: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn a_read_only_stdout_is_a_delivery_failure_not_a_silent_success() -> Result<(), String> {
    for args in [vec!["version", "--json"], vec!["hosts", "--json"]] {
        let harness = Harness::open("read-only-stdout")?;
        let sink = std::fs::File::open("/dev/null").map_err(|error| fail("open sink", error))?;
        let stderr_path = harness.path("stderr");
        let mut child = harness
            .command(&args)?
            .stdout(Stdio::from(sink))
            .stderr(Stdio::from(
                std::fs::File::create(&stderr_path).map_err(|e| fail("open sink", e))?,
            ))
            .spawn()
            .map_err(|error| fail("spawn candidate", error))?;
        let status = child
            .wait()
            .map_err(|error| fail("wait for candidate", error))?;
        let stderr =
            std::fs::read_to_string(&stderr_path).map_err(|error| fail("read stderr", error))?;
        assert_eq!(
            status.code().unwrap_or(-1),
            255,
            "a document that could not be written is not a success: {args:?}"
        );
        assert!(
            stderr.contains("OUTPUT_WRITE_FAILED"),
            "{args:?} lost stdout said: {stderr:?}"
        );
    }
    Ok(())
}

#[test]
fn an_unanswered_run_is_never_a_success_and_never_a_silent_zero() -> Result<(), String> {
    let harness = Harness::open("zero-status")?;
    harness.stub("ssh", "exit 0")?;
    let (outcome, value) = envelope(
        &harness,
        &["exec", "gpu", "--json", "--fresh", "--command", "true"],
    )?;
    assert_eq!(
        outcome.status, 255,
        "a zero from ssh without completion is not the remote's zero"
    );
    want_code(&value, "REMOTE_EXECUTION_UNKNOWN")?;
    assert_eq!(value["error"]["retryable"], serde_json::Value::Bool(false));
    assert_eq!(
        value
            .pointer("/data/execution/status")
            .and_then(serde_json::Value::as_str),
        Some("unknown")
    );
    assert!(
        value
            .pointer("/data/execution")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|execution| !execution.contains_key("exit_code")),
        "an unknown exit carries no integer (WIRE-005): {}",
        value["data"]
    );
    Ok(())
}

#[test]
fn a_readable_command_file_is_the_exact_program_that_is_submitted() -> Result<(), String> {
    let harness = Harness::open("command-file-ok")?;
    harness.stub("ssh", "exit 255")?;
    harness.write("home/program.sh", "printf marker\n", None)?;
    let file = harness.path("home/program.sh");
    let name = file.to_str().ok_or("fixture path is not UTF-8")?;
    let (_, value) = envelope(&harness, &["exec", "gpu", "--json", "--command-file", name])?;
    // The stub never answers, so this can only be execution uncertainty — but
    // the file must have been accepted as the program.
    want_code(&value, "REMOTE_EXECUTION_UNKNOWN")?;
    assert_eq!(
        harness.calls().len(),
        1,
        "a readable file must be accepted as the program, not refused locally: {}",
        harness.calls_text()
    );
    assert!(
        !harness.calls_text().contains(name),
        "the local path is not part of what is sent"
    );
    Ok(())
}
