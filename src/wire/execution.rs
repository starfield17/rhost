//! The `exec` envelope: execution state, bounded output, and the failure
//! taxonomy a caller branches on.

use super::{Envelope, error};
use crate::domain::{
    self, CancelSignal, CleanupEvidence, ExecFailure, Interruption, PreExecFailure,
};
use serde::Serialize;
use std::borrow::Cow;

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExecutionDto {
    Completed { exit_code: u8 },
    Unknown,
    NotStarted,
}
impl From<domain::Execution> for ExecutionDto {
    fn from(value: domain::Execution) -> Self {
        match value {
            domain::Execution::Completed(code) => Self::Completed {
                exit_code: code.get(),
            },
            domain::Execution::Unknown => Self::Unknown,
            domain::Execution::NotStarted => Self::NotStarted,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct CaptureDto<'a> {
    content: Cow<'a, str>,
    bytes: u64,
    truncated: bool,
}
impl<'a> From<&'a domain::CapturedText> for CaptureDto<'a> {
    fn from(value: &'a domain::CapturedText) -> Self {
        Self {
            content: value.content(),
            bytes: value.bytes(),
            truncated: value.truncated(),
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputDto<'a> {
    Streams {
        stdout: CaptureDto<'a>,
        stderr: CaptureDto<'a>,
    },
    Pty {
        content: Cow<'a, str>,
        bytes: u64,
        truncated: bool,
    },
}
#[derive(Debug, Serialize)]
pub struct CleanupDto {
    status: &'static str,
}
#[derive(Debug, Serialize)]
pub struct ExecDto<'a> {
    execution: ExecutionDto,
    cleanup: CleanupDto,
    output: OutputDto<'a>,
    duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    cancel_signal: Option<&'static str>,
}

/// The stable code and message an exec outcome carries, factored out so the
/// human status line and the JSON error payload cannot drift apart (AGENTS.md §6).
pub fn exec_diagnostic(result: &domain::ExecOutcome) -> Option<(&'static str, &'static str)> {
    let (code, message, _) = failure_parts(result.failure()?)?;
    Some((code, message))
}

fn failure_parts(reason: domain::ExecFailure) -> Option<(&'static str, &'static str, bool)> {
    Some(match reason {
        ExecFailure::Interrupted(Interruption::Timeout) => (
            "REMOTE_COMMAND_TIMEOUT",
            "deadline exceeded; side effects may remain",
            false,
        ),
        ExecFailure::Interrupted(Interruption::Cancelled(_)) => (
            "REMOTE_COMMAND_CANCELLED",
            "local invocation cancelled; side effects may remain",
            false,
        ),
        ExecFailure::ExecutionUnknown => (
            "REMOTE_EXECUTION_UNKNOWN",
            "completion evidence is missing",
            false,
        ),
        ExecFailure::OutputWriteFailed => (
            "OUTPUT_WRITE_FAILED",
            "command output delivery failed",
            false,
        ),
        ExecFailure::BeforeSubmission(reason) => match reason {
            PreExecFailure::Usage => ("USAGE_ERROR", "invalid command arguments", false),
            PreExecFailure::Config => ("CONFIG_INVALID", "invalid configuration", false),
            PreExecFailure::HostUnknown => ("HOST_UNKNOWN", "target could not be resolved", false),
            PreExecFailure::Authentication => {
                ("SSH_AUTH_FAILED", "OpenSSH authentication failed", false)
            }
            PreExecFailure::HostKey => (
                "HOST_KEY_FAILED",
                "OpenSSH host key verification failed",
                false,
            ),
            PreExecFailure::Connection => (
                "SSH_UNREACHABLE",
                "connection failed before command submission",
                true,
            ),
            PreExecFailure::DependencyMissing => (
                "REMOTE_DEPENDENCY_MISSING",
                "required remote dependency is unavailable",
                false,
            ),
        },
    })
}

pub fn exec<'a>(host: &str, result: &'a domain::ExecOutcome) -> Envelope<ExecDto<'a>> {
    let failure = result
        .failure()
        .and_then(failure_parts)
        .map(|(code, message, retryable)| error(code, message, retryable));
    let signal = match result.failure() {
        Some(ExecFailure::Interrupted(Interruption::Cancelled(CancelSignal::Int))) => {
            Some("SIGINT")
        }
        Some(ExecFailure::Interrupted(Interruption::Cancelled(CancelSignal::Term))) => {
            Some("SIGTERM")
        }
        _ => None,
    };
    Envelope {
        schema_version: 2,
        operation: "exec",
        ok: failure.is_none(),
        host: Some(host.into()),
        error: failure,
        data: ExecDto {
            execution: result.execution().into(),
            cleanup: CleanupDto {
                status: match result.cleanup() {
                    CleanupEvidence::NotAttempted => "not_attempted",
                    CleanupEvidence::ConfirmedStopped => "confirmed_stopped",
                    CleanupEvidence::Unconfirmed => "unconfirmed",
                },
            },
            output: OutputDto::Streams {
                stdout: (&result.output().stdout).into(),
                stderr: (&result.output().stderr).into(),
            },
            duration_ms: result.duration_ms(),
            cancel_signal: signal,
        },
    }
}
