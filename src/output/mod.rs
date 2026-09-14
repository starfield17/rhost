//! Explicit schema-v2 DTO mapping. Domain values never derive Serialize.
use crate::domain::{
    self, CancelSignal, CleanupEvidence, ExecFailure, Interruption, PreExecFailure, SessionFailure,
};
use serde::Serialize;
use std::borrow::Cow;
use std::io::{self, Write};

#[derive(Debug, Serialize)]
pub struct Envelope<T> {
    schema_version: u8,
    operation: &'static str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    data: T,
    error: Option<ErrorPayload>,
}
impl<T: Serialize> Envelope<T> {
    /// Delivery is attempted once. A broken sink cannot carry its own error
    /// envelope; the CLI reports that failure on stderr and exits nonzero.
    pub fn write(&self, sink: &mut impl Write) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        bytes.push(b'\n');
        sink.write_all(&bytes)
    }
}
#[derive(Debug, Serialize)]
pub struct ErrorPayload {
    code: &'static str,
    message: String,
    retryable: bool,
}
fn error(code: &'static str, message: &str, retryable: bool) -> ErrorPayload {
    ErrorPayload {
        code,
        message: message.into(),
        retryable,
    }
}
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

pub fn exec<'a>(host: &str, result: &'a domain::ExecOutcome) -> Envelope<ExecDto<'a>> {
    let failure = result.failure().map(|reason| match reason {
        ExecFailure::Interrupted(Interruption::Timeout) => error(
            "REMOTE_COMMAND_TIMEOUT",
            "deadline exceeded; side effects may remain",
            false,
        ),
        ExecFailure::Interrupted(Interruption::Cancelled(_)) => error(
            "REMOTE_COMMAND_CANCELLED",
            "local invocation cancelled; side effects may remain",
            false,
        ),
        ExecFailure::ExecutionUnknown => error(
            "REMOTE_EXECUTION_UNKNOWN",
            "completion evidence is missing",
            false,
        ),
        ExecFailure::OutputWriteFailed => error(
            "OUTPUT_WRITE_FAILED",
            "command output delivery failed",
            false,
        ),
        ExecFailure::BeforeSubmission(reason) => match reason {
            PreExecFailure::Usage => error("USAGE_ERROR", "invalid command arguments", false),
            PreExecFailure::Config => error("CONFIG_INVALID", "invalid configuration", false),
            PreExecFailure::HostUnknown => {
                error("HOST_UNKNOWN", "target could not be resolved", false)
            }
            PreExecFailure::Authentication => {
                error("SSH_AUTH_FAILED", "OpenSSH authentication failed", false)
            }
            PreExecFailure::HostKey => error(
                "HOST_KEY_FAILED",
                "OpenSSH host key verification failed",
                false,
            ),
            PreExecFailure::Connection => error(
                "SSH_UNREACHABLE",
                "connection failed before command submission",
                true,
            ),
            PreExecFailure::DependencyMissing => error(
                "REMOTE_DEPENDENCY_MISSING",
                "required remote dependency is unavailable",
                false,
            ),
        },
    });
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
    let failure = result.failure().map(|reason| match reason {
        SessionFailure::WriterBusy => error(
            "SESSION_UNHEALTHY",
            "another session writer holds the lock",
            true,
        ),
        SessionFailure::Busy => error("SESSION_BUSY", "managed shell does not own the pane", true),
        SessionFailure::Timeout => error(
            "REMOTE_COMMAND_TIMEOUT",
            "session command deadline exceeded",
            false,
        ),
        SessionFailure::Unhealthy => error(
            "SESSION_UNHEALTHY",
            "session completion evidence is invalid or missing",
            false,
        ),
        SessionFailure::NotFound => {
            error("SESSION_NOT_FOUND", "session could not be resolved", false)
        }
    });
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

#[derive(Debug, Serialize)]
pub struct VersionDto {
    version: &'static str,
    commit: &'static str,
    build_date: &'static str,
    schema_version: u8,
}
pub fn version() -> Envelope<VersionDto> {
    Envelope {
        schema_version: 2,
        operation: "version",
        ok: true,
        host: None,
        error: None,
        data: VersionDto {
            version: env!("CARGO_PKG_VERSION"),
            commit: option_env!("RHOST_BUILD_COMMIT").unwrap_or("none"),
            build_date: option_env!("RHOST_BUILD_DATE").unwrap_or("unknown"),
            schema_version: 2,
        },
    }
}
pub fn usage(message: &str) -> Envelope<Option<()>> {
    Envelope {
        schema_version: 2,
        operation: "usage",
        ok: false,
        host: None,
        data: None,
        error: Some(error("USAGE_ERROR", message, false)),
    }
}
