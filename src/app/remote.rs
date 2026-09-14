//! The one path out to a host, and the codes an answer comes back in.
//!
//! Every use-case that submits a program — a file helper, a transfer probe, a
//! session helper — submits it through [`run_remote`], so a transport failure is
//! classified exactly once: the same cause reports the same code whether the
//! caller asked about a file or a session.

use super::Error;
use super::exec::{self, Request as ExecRequest};
use crate::domain::{ExecFailure, ExecOutcome, Execution, Interruption, PreExecFailure};
use crate::transport::openssh::Client;
use std::time::Duration;

pub(crate) fn internal(message: impl Into<String>) -> Error {
    Error::new("INTERNAL", message)
}

/// Maps a failed execution onto the taxonomy. `None` means the command ran and
/// its status is the caller's to interpret.
fn submission(outcome: &ExecOutcome) -> Option<Error> {
    let failure = outcome.failure()?;
    Some(match failure {
        ExecFailure::Interrupted(Interruption::Timeout) => Error::new(
            "REMOTE_COMMAND_TIMEOUT",
            "the operation exceeded its timeout; remote state is unknown",
        ),
        ExecFailure::Interrupted(Interruption::Cancelled(_)) => Error::new(
            "REMOTE_COMMAND_CANCELLED",
            "the local invocation was cancelled; remote state is unknown",
        ),
        ExecFailure::ExecutionUnknown => {
            Error::new("REMOTE_EXECUTION_UNKNOWN", "completion evidence is missing")
        }
        ExecFailure::OutputWriteFailed => {
            Error::new("OUTPUT_WRITE_FAILED", "command output delivery failed")
        }
        ExecFailure::BeforeSubmission(reason) => match reason {
            PreExecFailure::Usage => Error::new("USAGE_ERROR", "invalid arguments"),
            PreExecFailure::Config => Error::new("CONFIG_INVALID", "invalid configuration"),
            PreExecFailure::HostUnknown => {
                Error::new("HOST_UNKNOWN", "target could not be resolved")
            }
            PreExecFailure::Authentication => {
                Error::new("SSH_AUTH_FAILED", "OpenSSH authentication failed")
            }
            PreExecFailure::HostKey => {
                Error::new("HOST_KEY_FAILED", "OpenSSH host key verification failed")
            }
            PreExecFailure::Connection => Error::new(
                "SSH_UNREACHABLE",
                "connection failed before the command was submitted",
            )
            .retryable(),
            PreExecFailure::DependencyMissing => Error::new(
                "REMOTE_DEPENDENCY_MISSING",
                "a required remote command is unavailable",
            ),
        },
    })
}

/// Runs one remote command through the normal execution path and reports its
/// status with the streams it produced.
pub(crate) fn run_remote(
    client: &Client,
    host: &str,
    command: &str,
    stdin: Option<&[u8]>,
    timeout: Duration,
    capture: usize,
) -> Result<(u8, String, String), Error> {
    let request = ExecRequest {
        host,
        command,
        cwd: None,
        env: &[],
        timeout: Some(timeout),
        capture: Some(capture),
        fresh: false,
    };
    let outcome = exec::execute_captured(client, command, &request, stdin)
        .map_err(|error| internal(error.to_string()))?;
    if let Some(error) = submission(&outcome) {
        return Err(error);
    }
    let code = match outcome.execution() {
        Execution::Completed(code) => code.get(),
        // `submission` above covers every state without a completion, so this is
        // unreachable rather than an assumed success.
        Execution::Unknown | Execution::NotStarted => {
            return Err(Error::new(
                "REMOTE_EXECUTION_UNKNOWN",
                "completion evidence is missing",
            ));
        }
    };
    let output = outcome.output();
    Ok((
        code,
        output.stdout.content().to_string(),
        output.stderr.content().to_string(),
    ))
}
