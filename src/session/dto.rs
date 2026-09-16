//! The schema-v2 view of a session.
//!
//! Two of these carry partial data on failure, because the session's *state* is
//! the answer: an exec that timed out and a recover that found a program still
//! holding the pane both have to say what the session is like now, not merely
//! that the call failed.

use super::ops as app_session;
use crate::domain::{self, SessionFailure};
use crate::remote::Error;
use crate::wire::execution::{ExecutionDto, OutputDto};
use crate::wire::{Envelope, error, failure};
use serde::Serialize;

/// One session record. `session_id` is the canonical identity; the name a caller
/// used is a separate field because it is not one (SESSION-007).
#[derive(Debug, Serialize)]
pub struct SessionInfoDto<'a> {
    session_id: &'a str,
    name: &'a str,
    tmux_session: &'a str,
    created_at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_cwd: Option<&'a str>,
    shell: &'a str,
    status: &'a str,
}

impl<'a> From<&'a app_session::Info> for SessionInfoDto<'a> {
    fn from(value: &'a app_session::Info) -> Self {
        Self {
            session_id: &value.id,
            name: &value.name,
            tmux_session: &value.tmux_session,
            created_at: &value.created_at,
            // A session created without a cwd has none to report; an empty
            // string would claim one.
            initial_cwd: (!value.initial_cwd.is_empty()).then_some(value.initial_cwd.as_str()),
            shell: &value.shell,
            status: value.status.as_str(),
        }
    }
}

/// The failure shape of `session.create`: the candidate identity this invocation
/// reserved, and what is known about whether the remote created it. A caller that
/// lost the answer uses `session_id` with `session list` instead of retrying
/// blind (SESSION-010).
#[derive(Debug, Serialize)]
pub struct SessionCreateFailureDto<'a> {
    session_id: &'a str,
    session_ref: &'a str,
    creation_status: &'static str,
}

pub fn session_created<'a>(
    host: &str,
    value: &'a app_session::Created,
) -> Envelope<SessionInfoDto<'a>> {
    Envelope {
        schema_version: 2,
        operation: "session.create",
        ok: true,
        host: Some(host.into()),
        // A successful create always carries the record.
        data: SessionInfoDto::from(
            value
                .info
                .as_ref()
                .unwrap_or_else(|| unreachable!("successful create has an Info")),
        ),
        error: None,
    }
}

pub fn session_create_failure<'a>(
    host: &str,
    value: &'a app_session::Created,
) -> Envelope<SessionCreateFailureDto<'a>> {
    let error = value
        .error
        .as_ref()
        .map(|failure| error(failure.code, &failure.message, failure.retryable))
        .unwrap_or_else(|| error("INTERNAL", "missing create failure", false));
    Envelope {
        schema_version: 2,
        operation: "session.create",
        ok: false,
        host: Some(host.into()),
        data: SessionCreateFailureDto {
            session_id: &value.candidate_id,
            session_ref: &value.candidate_ref,
            creation_status: value.creation_status.as_str(),
        },
        error: Some(error),
    }
}

#[derive(Debug, Serialize)]
pub struct SessionsDto<'a> {
    sessions: Vec<SessionInfoDto<'a>>,
}

pub fn sessions<'a>(host: &str, rows: &'a [app_session::Info]) -> Envelope<SessionsDto<'a>> {
    Envelope {
        schema_version: 2,
        operation: "session.list",
        ok: true,
        host: Some(host.into()),
        data: SessionsDto {
            sessions: rows.iter().map(SessionInfoDto::from).collect(),
        },
        error: None,
    }
}

#[derive(Debug, Serialize)]
pub struct SessionExecDto<'a> {
    session_id: Option<&'a str>,
    session_ref: &'a str,
    execution: ExecutionDto,
    output: OutputDto<'a>,
    session_preserved: bool,
}

pub fn session_exec<'a>(
    host: &str,
    result: &'a domain::SessionExecOutcome,
) -> Envelope<SessionExecDto<'a>> {
    let failure = session_exec_failure(result)
        .map(|(code, message, retryable)| error(code, &message, retryable));
    let capture = &result.output().0;
    Envelope {
        schema_version: 2,
        operation: "session.exec",
        ok: failure.is_none(),
        host: Some(host.into()),
        error: failure,
        data: SessionExecDto {
            session_id: result.id().map(domain::SessionId::as_str),
            session_ref: result.reference().as_str(),
            execution: result.execution().into(),
            session_preserved: result.preserved(),
            output: OutputDto::Pty {
                content: capture.content(),
                bytes: capture.bytes(),
                truncated: capture.truncated(),
            },
        },
    }
}

/// The stable code, message and retryability one session outcome reports.
///
/// It is one function because both renderings read it: the envelope above and the
/// human status line, which must not be able to disagree about why a command did
/// not run.
pub fn session_exec_failure(
    result: &domain::SessionExecOutcome,
) -> Option<(&'static str, String, bool)> {
    result.failure().map(|reason| match reason {
        SessionFailure::WriterBusy => (
            "SESSION_UNHEALTHY",
            "another session writer holds the lock".to_string(),
            true,
        ),
        SessionFailure::Busy => ("SESSION_BUSY", busy_message(result.foreground()), true),
        SessionFailure::Timeout => (
            "REMOTE_COMMAND_TIMEOUT",
            "session command deadline exceeded".to_string(),
            false,
        ),
        SessionFailure::Unhealthy => (
            "SESSION_UNHEALTHY",
            "session completion evidence is invalid or missing".to_string(),
            false,
        ),
        SessionFailure::NotFound => (
            "SESSION_NOT_FOUND",
            "session could not be resolved".to_string(),
            false,
        ),
    })
}

/// What the process exits with: the remote command's own status when it
/// completed, the timeout status when the deadline was hit, and the adapter's
/// failure status otherwise (WIRE-004).
pub fn session_exec_status(result: &domain::SessionExecOutcome) -> u8 {
    if let Some((code, _, _)) = session_exec_failure(result) {
        return if code == "REMOTE_COMMAND_TIMEOUT" {
            124
        } else {
            255
        };
    }
    match result.execution() {
        domain::Execution::Completed(code) => code.get(),
        domain::Execution::Unknown | domain::Execution::NotStarted => 255,
    }
}

/// A refusal has to name what owns the pane: the caller's next move is to drive
/// or interrupt *that* program, so "something else" is not actionable.
fn busy_message(foreground: Option<&str>) -> String {
    let what = foreground.unwrap_or("another program");
    format!(
        "session foreground is {what}, not the managed shell; use session send/read, or session recover"
    )
}

#[derive(Debug, Serialize)]
pub struct SessionReadDto<'a> {
    session_id: &'a str,
    session_ref: &'a str,
    content: &'a str,
    encoding: &'static str,
    from: u64,
    next: u64,
    more: bool,
}

pub fn session_read<'a>(host: &str, result: &'a app_session::Read) -> Envelope<SessionReadDto<'a>> {
    Envelope {
        schema_version: 2,
        operation: "session.read",
        ok: true,
        host: Some(host.into()),
        data: SessionReadDto {
            session_id: &result.id,
            session_ref: &result.reference,
            content: &result.content,
            encoding: "utf-8",
            from: result.from,
            next: result.next,
            more: result.more(),
        },
        error: None,
    }
}

#[derive(Debug, Serialize)]
pub struct SessionRecoverDto<'a> {
    session_id: Option<&'a str>,
    session_ref: &'a str,
    session_preserved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreground: Option<&'a str>,
}

/// Recovery reports its own state on both paths: whether the shell came back is
/// the answer, and a failure without it would be a failure to say.
pub fn session_recover<'a>(
    host: &str,
    result: &'a app_session::Recover,
) -> Envelope<SessionRecoverDto<'a>> {
    let data = SessionRecoverDto {
        session_id: result.id.as_deref(),
        session_ref: &result.reference,
        session_preserved: result.preserved,
        foreground: result.foreground.as_deref(),
    };
    match &result.failure {
        None => Envelope {
            schema_version: 2,
            operation: "session.recover",
            ok: true,
            host: Some(host.into()),
            data,
            error: None,
        },
        Some(failure) => Envelope {
            schema_version: 2,
            operation: "session.recover",
            ok: false,
            host: Some(host.into()),
            data,
            error: Some(error(failure.code, &failure.message, failure.retryable)),
        },
    }
}

#[derive(Debug, Serialize)]
pub struct SessionSentDto {
    sent: bool,
}

/// The injected data may be sensitive, so the answer never echoes it: `sent` is
/// the whole claim, and it is only made after the helper confirmed the paste.
pub fn session_sent(host: &str) -> Envelope<SessionSentDto> {
    Envelope {
        schema_version: 2,
        operation: "session.send",
        ok: true,
        host: Some(host.into()),
        data: SessionSentDto { sent: true },
        error: None,
    }
}

#[derive(Debug, Serialize)]
pub struct SessionClosedDto {
    closed: bool,
}

pub fn session_closed(host: &str) -> Envelope<SessionClosedDto> {
    Envelope {
        schema_version: 2,
        operation: "session.close",
        ok: true,
        host: Some(host.into()),
        data: SessionClosedDto { closed: true },
        error: None,
    }
}

/// A session failure that carries no partial state, phrased once so the code and
/// its retryability cannot drift between call sites.
pub fn session_failure(
    operation: &'static str,
    host: &str,
    reason: &Error,
) -> Envelope<Option<()>> {
    failure(
        operation,
        host,
        reason.code,
        &reason.message,
        reason.retryable,
    )
}
