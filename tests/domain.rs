use rhost::domain::*;
use rhost::wire;
use serde_json::Value;

fn streams() -> Result<ProcessOutput, DomainError> {
    Ok(ProcessOutput {
        stdout: CapturedText::new(String::new(), 0)?,
        stderr: CapturedText::new(String::new(), 0)?,
    })
}

#[test]
fn completion_and_interruption_are_independent() -> Result<(), Box<dyn std::error::Error>> {
    for (reason, expected) in [
        (Interruption::Timeout, 124),
        (Interruption::Cancelled(CancelSignal::Int), 130),
        (Interruption::Cancelled(CancelSignal::Term), 143),
    ] {
        let result = ExecOutcome::interrupted(
            CompletionEvidence::Completed(completion(7)?),
            reason,
            CleanupEvidence::NotAttempted,
            streams()?,
            10,
        )?;
        assert_eq!(result.process_status(), expected);
        let wire = serde_json::to_value(wire::exec("gpu", &result))?;
        assert_eq!(wire["ok"], false);
        assert_eq!(wire["data"]["execution"]["status"], "completed");
        assert_eq!(wire["data"]["execution"]["exit_code"], 7);
        assert_eq!(wire["error"]["retryable"], false);
    }
    assert!(
        ExecOutcome::interrupted(
            CompletionEvidence::Completed(completion(0)?),
            Interruption::Timeout,
            CleanupEvidence::ConfirmedStopped,
            streams()?,
            10
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn cleanup_is_not_permission_to_retry() -> Result<(), Box<dyn std::error::Error>> {
    for cleanup in [
        CleanupEvidence::NotAttempted,
        CleanupEvidence::ConfirmedStopped,
        CleanupEvidence::Unconfirmed,
    ] {
        let result = ExecOutcome::interrupted(
            CompletionEvidence::Missing,
            Interruption::Timeout,
            cleanup,
            streams()?,
            10,
        )?;
        let wire = serde_json::to_value(wire::exec("gpu", &result))?;
        assert_eq!(wire["error"]["retryable"], false);
        assert_eq!(wire["data"]["execution"]["status"], "unknown");
        assert!(wire["data"]["execution"].get("exit_code").is_none());
    }
    Ok(())
}

#[test]
fn remote_nonzero_is_completed_and_delivery_failure_preserves_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let code = completion(255)?;
    let result = ExecOutcome::completed(code, streams()?, 0);
    assert_eq!(result.process_status(), 255);
    assert_eq!(
        serde_json::to_value(wire::exec("gpu", &result))?["ok"],
        true
    );
    let failed = ExecOutcome::output_failed(
        CompletionEvidence::Completed(completion(255)?),
        CleanupEvidence::Unconfirmed,
        streams()?,
        0,
    );
    let wire = serde_json::to_value(wire::exec("gpu", &failed))?;
    assert_eq!(wire["error"]["code"], "OUTPUT_WRITE_FAILED");
    assert_eq!(wire["data"]["execution"]["exit_code"], 255);
    Ok(())
}

#[test]
fn identities_paths_and_hashes_are_validated() -> Result<(), DomainError> {
    for code in [-1, 256] {
        assert!(ExitCode::new(code).is_err());
    }
    assert!(SessionId::new("../other".into()).is_err());
    assert!(RemotePath::new("'~/file'".into()).is_err());
    assert!(RemotePath::new("\"~/file\"".into()).is_err());
    assert!(RemotePath::new("bad\0path".into()).is_err());
    assert_eq!(
        RemotePath::new("~/a'b;$(literal)".into())?.as_str(),
        "~/a'b;$(literal)"
    );
    assert!(Sha256::new("bad".into()).is_err());
    assert!(InvocationToken::new("bad".into()).is_err());
    let token = InvocationToken::new("a".repeat(32))?;
    assert!(token.matches(&"a".repeat(32)));
    assert!(!token.matches(&"b".repeat(32)));
    assert!(CapturedText::new("hello".into(), 1).is_err());
    assert!(CapturedText::new("é".into(), 3)?.truncated());
    Ok(())
}

#[test]
fn busy_session_has_no_exit_and_no_stdout_alias() -> Result<(), Box<dyn std::error::Error>> {
    let result = SessionExecOutcome::busy(
        SessionId::new("canonical".into())?,
        SessionRef::new("caller-name".into())?,
        PtyOutput(CapturedText::new(String::new(), 0)?),
    );
    let wire = serde_json::to_value(rhost::session::session_exec("gpu", &result))?;
    assert_eq!(wire["data"]["session_id"], "canonical");
    assert_eq!(wire["data"]["session_ref"], "caller-name");
    assert_eq!(wire["data"]["execution"]["status"], "not_started");
    assert!(wire["data"]["execution"].get("exit_code").is_none());
    assert_eq!(wire["data"]["output"]["kind"], "pty");
    assert!(wire["data"].get("stdout").is_none());
    assert_eq!(wire["error"]["code"], "SESSION_BUSY");
    Ok(())
}

#[test]
fn dto_matches_independent_contract_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let fixtures: Vec<Value> = serde_json::from_str(include_str!("fixtures/result-v2-valid.json"))?;
    let fixture = fixtures
        .iter()
        .find(|f| f["name"] == "exec")
        .ok_or("missing exec fixture")?;
    let result = ExecOutcome::completed(completion(0)?, streams()?, 0);
    assert_eq!(
        serde_json::to_value(wire::exec("gpu", &result))?,
        fixture["result"]
    );
    Ok(())
}

#[test]
fn failed_sink_is_not_retried() -> Result<(), Box<dyn std::error::Error>> {
    struct BrokenSink(usize);
    impl std::io::Write for BrokenSink {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            self.0 += 1;
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = BrokenSink(0);
    assert!(wire::version().write(&mut sink).is_err());
    assert_eq!(sink.0, 1);
    Ok(())
}

#[test]
fn invalid_utf8_does_not_corrupt_source_byte_evidence() -> Result<(), DomainError> {
    let capture = CapturedText::from_bytes(vec![0xff], 1)?;
    assert_eq!(capture.content(), "�");
    assert_eq!(capture.bytes(), 1);
    assert!(!capture.truncated());
    Ok(())
}

fn completion(code: i32) -> Result<VerifiedCompletion, DomainError> {
    InvocationToken::new("a".repeat(32))?.verify_completion(&"a".repeat(32), ExitCode::new(code)?)
}

#[test]
fn foreign_completion_cannot_construct_verified_evidence() -> Result<(), DomainError> {
    let expected = InvocationToken::new("a".repeat(32))?;
    assert!(
        expected
            .verify_completion(&"b".repeat(32), ExitCode::new(0)?)
            .is_err()
    );
    Ok(())
}
