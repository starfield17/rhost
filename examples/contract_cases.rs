//! Actual DTO cases for independent JSON Schema validation.
use rhost::{
    domain::*,
    session::{Created, CreationStatus},
    wire,
};
use serde::Serialize;
fn emit<T: Serialize>(
    rows: &mut Vec<serde_json::Value>,
    value: T,
) -> Result<(), Box<dyn std::error::Error>> {
    rows.push(serde_json::to_value(value)?);
    Ok(())
}
fn streams() -> Result<ProcessOutput, DomainError> {
    Ok(ProcessOutput {
        stdout: CapturedText::from_bytes(vec![0xff, b'a'], 5)?,
        stderr: CapturedText::new(String::new(), 0)?,
    })
}
fn pty() -> Result<PtyOutput, DomainError> {
    Ok(PtyOutput(CapturedText::new(String::new(), 0)?))
}
pub fn cases() -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let mut rows = Vec::new();
    emit(&mut rows, wire::version())?;
    emit(&mut rows, wire::usage("missing command"))?;
    for code in [0, 7, 255] {
        emit(
            &mut rows,
            wire::exec(
                "gpu",
                &ExecOutcome::completed(completion(code)?, streams()?, 0),
            ),
        )?;
    }
    for cleanup in [
        CleanupEvidence::NotAttempted,
        CleanupEvidence::ConfirmedStopped,
        CleanupEvidence::Unconfirmed,
    ] {
        for reason in [
            Interruption::Timeout,
            Interruption::Cancelled(CancelSignal::Int),
            Interruption::Cancelled(CancelSignal::Term),
        ] {
            emit(
                &mut rows,
                wire::exec(
                    "gpu",
                    &ExecOutcome::interrupted(
                        CompletionEvidence::Missing,
                        reason,
                        cleanup,
                        streams()?,
                        1,
                    )?,
                ),
            )?;
        }
        emit(
            &mut rows,
            wire::exec(
                "gpu",
                &ExecOutcome::output_failed(
                    CompletionEvidence::Completed(completion(7)?),
                    cleanup,
                    streams()?,
                    1,
                ),
            ),
        )?;
    }
    for reason in [
        Interruption::Timeout,
        Interruption::Cancelled(CancelSignal::Int),
        Interruption::Cancelled(CancelSignal::Term),
    ] {
        emit(
            &mut rows,
            wire::exec(
                "gpu",
                &ExecOutcome::interrupted(
                    CompletionEvidence::Completed(completion(7)?),
                    reason,
                    CleanupEvidence::NotAttempted,
                    streams()?,
                    1,
                )?,
            ),
        )?;
    }
    emit(
        &mut rows,
        wire::exec("gpu", &ExecOutcome::unknown(streams()?, 1)),
    )?;
    for reason in [
        PreExecFailure::Usage,
        PreExecFailure::Config,
        PreExecFailure::HostUnknown,
        PreExecFailure::Authentication,
        PreExecFailure::HostKey,
        PreExecFailure::Connection,
        PreExecFailure::DependencyMissing,
    ] {
        emit(
            &mut rows,
            wire::exec("gpu", &ExecOutcome::not_started(reason, streams()?, 0)),
        )?;
    }
    let id = || SessionId::new("canonical-id".into());
    let reference = || SessionRef::new("caller-name".into());
    emit(
        &mut rows,
        rhost::session::session_exec(
            "gpu",
            &SessionExecOutcome::completed(id()?, reference()?, completion(7)?, pty()?),
        ),
    )?;
    emit(
        &mut rows,
        rhost::session::session_exec(
            "gpu",
            &SessionExecOutcome::busy(id()?, reference()?, pty()?),
        ),
    )?;
    emit(
        &mut rows,
        rhost::session::session_exec(
            "gpu",
            &SessionExecOutcome::writer_busy(id()?, reference()?, pty()?),
        ),
    )?;
    emit(
        &mut rows,
        rhost::session::session_exec("gpu", &SessionExecOutcome::not_found(reference()?, pty()?)),
    )?;
    emit(
        &mut rows,
        rhost::session::session_exec(
            "gpu",
            &SessionExecOutcome::unhealthy(None, reference()?, pty()?),
        ),
    )?;
    for preserved in [false, true] {
        emit(
            &mut rows,
            rhost::session::session_exec(
                "gpu",
                &SessionExecOutcome::timed_out(
                    id()?,
                    reference()?,
                    CompletionEvidence::Missing,
                    preserved,
                    pty()?,
                ),
            ),
        )?;
    }
    for status in [
        CreationStatus::NotCreated,
        CreationStatus::Unknown,
        CreationStatus::Created,
    ] {
        emit(
            &mut rows,
            rhost::session::session_create_failure(
                "gpu",
                &Created::failed(
                    "s_candidate",
                    "caller-name",
                    status,
                    "SESSION_UNHEALTHY",
                    "completion evidence is missing",
                    false,
                ),
            ),
        )?;
    }
    Ok(rows)
}

fn completion(code: i32) -> Result<VerifiedCompletion, DomainError> {
    InvocationToken::new("a".repeat(32))?.verify_completion(&"a".repeat(32), ExitCode::new(code)?)
}

#[cfg(not(test))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for row in cases()? {
        println!("{}", row);
    }
    Ok(())
}
