//! `exec`: run one command in a session and say what the pane did.

use super::errors::run_helper;
use super::shared::FIELD_CAPTURE;
use crate::app::Error;
use crate::app::remote::internal;
use crate::domain::{
    CapturedText, CompletionEvidence, ExitCode, InvocationToken, PtyOutput, SessionExecOutcome,
    SessionId, SessionRef,
};
use crate::session::{self, ExecResult, HelperFailure};
use crate::shell;
use crate::transport::Client;
use std::time::Duration;

/// What a caller passes to `session exec`.
pub struct ExecOptions<'a> {
    pub host: &'a str,
    pub session: &'a str,
    pub command: &'a str,
    pub timeout: Duration,
}

/// Runs one command in a session.
///
/// The outcome carries both the result and any session-level failure, because a
/// refusal still has to describe the session: a timeout says whether the pane came
/// back, and a busy pane says what is holding it. Only a transport failure is an
/// `Err`, and then there is no session state to report.
pub fn exec(client: &Client, options: &ExecOptions<'_>) -> Result<SessionExecOutcome, Error> {
    let reference = SessionRef::new(options.session.to_string())
        .map_err(|error| Error::new("CONFIG_INVALID", error.to_string()))?;
    if options.command.trim().is_empty() {
        return Err(Error::new("CONFIG_INVALID", "no command given"));
    }
    let token = session::new_token()
        .map_err(|error| internal(format!("could not generate a submission token: {error}")))?;
    let script = session::exec_script(options.session, options.command, options.timeout, &token);
    let stdout = run_helper(
        client,
        options.host,
        &script,
        options.timeout,
        FIELD_CAPTURE,
    )?;
    Ok(exec_outcome(&stdout, &token, reference))
}

/// Turns one helper answer into the domain outcome, so the wire mapping has a
/// single input.
fn exec_outcome(stdout: &str, token: &str, reference: SessionRef) -> SessionExecOutcome {
    match session::parse_exec(stdout, token) {
        ExecResult::Completed {
            id,
            output,
            exit_code,
        } => {
            let output = pty_output(&output);
            // An identity, a status or a token the domain refuses is not
            // completion: reporting success here would invent one of them.
            let Some(id) = SessionId::new(id).ok() else {
                return SessionExecOutcome::unhealthy(None, reference, output);
            };
            let (Ok(expected), Ok(code)) = (
                InvocationToken::new(token.to_string()),
                ExitCode::new(i32::from(exit_code)),
            ) else {
                return SessionExecOutcome::unhealthy(Some(id), reference, output);
            };
            match expected.verify_completion(token, code) {
                Ok(completion) => SessionExecOutcome::completed(id, reference, completion, output),
                Err(_) => SessionExecOutcome::unhealthy(Some(id), reference, output),
            }
        }
        ExecResult::Failed {
            failure,
            id,
            foreground,
            recovered,
        } => {
            let id = id.and_then(|raw| SessionId::new(raw).ok());
            let output = PtyOutput(CapturedText::empty());
            match failure {
                HelperFailure::Busy => match id {
                    Some(id) => SessionExecOutcome::busy_in(id, reference, foreground, output),
                    None => SessionExecOutcome::unhealthy(None, reference, output),
                },
                // Nothing was submitted in either case — the writer was refused,
                // or the paste never landed — so a retry cannot compound
                // anything.
                HelperFailure::Locked | HelperFailure::InputFailed => match id {
                    Some(id) => SessionExecOutcome::writer_busy(id, reference, output),
                    None => SessionExecOutcome::unhealthy(None, reference, output),
                },
                HelperFailure::Timeout => match id {
                    Some(id) => SessionExecOutcome::timed_out(
                        id,
                        reference,
                        CompletionEvidence::Missing,
                        recovered,
                        output,
                    ),
                    None => SessionExecOutcome::unhealthy(None, reference, output),
                },
                HelperFailure::NoSession | HelperFailure::SessionDied => {
                    SessionExecOutcome::not_found(reference, output)
                }
                _ => SessionExecOutcome::unhealthy(id, reference, output),
            }
        }
    }
}

/// Strips terminal sequences from raw pane bytes and counts the bytes that
/// actually arrived, so the envelope describes the source rather than the view.
fn pty_output(raw: &[u8]) -> PtyOutput {
    let text = shell::strip_ansi(&String::from_utf8_lossy(raw));
    PtyOutput(CapturedText::projected(
        text.into_bytes(),
        raw.len() as u64,
        false,
    ))
}
