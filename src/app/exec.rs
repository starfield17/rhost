//! Foreground execution: one shell program, one submission, honest evidence.
//!
//! This is the use-case layer. It decides what a run *means* and hands the CLI a
//! domain [`ExecOutcome`]; it starts no threads of its own and owns nothing
//! after it returns.
//!
//! Two shapes exist because two callers exist: [`execute_stream`] is the
//! command-line path, where bytes must be observable while the remote program
//! runs, and [`execute_captured`] is the helper path (doctor today; files and
//! sessions later), where the whole result is read once the child is gone.
//!
//! The pieces that are not the two entry points are their own files: `outcome`
//! decides what a finished run means and spends the invocation token, `capture`
//! turns bytes into a bounded [`crate::domain::ProcessOutput`], and `cleanup`
//! asks the remote host to stop a process group — reporting only what the
//! attempt proved.

mod capture;
mod cleanup;
mod outcome;

use crate::domain::{CancelSignal, ExecOutcome, InvocationToken, PreExecFailure, ProcessOutput};
use crate::shell;
use crate::transport::openssh::{Client, Streams};
use crate::transport::process::{Keep, StdinSource, Tap};
use crate::transport::protocol::{self, CompletionStream, ExecSpec, Separator};
use std::io::Write;
use std::time::Duration;

use capture::{process_output, truncate};
use outcome::{decide, realize};

/// Upper bound on one invocation's in-memory streams, even when a caller asks
/// for more: the cap bounds what rhost carries back, never what the remote
/// command may produce.
pub const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;

/// Bounds the best-effort remote process-group stop issued after an interruption.
pub const KILL_TIMEOUT: Duration = Duration::from_secs(8);

/// Default per-stream capture for the JSON path. An agent that asks for output
/// gets a bounded amount by default and can name a different one.
pub const DEFAULT_JSON_CAPTURE: usize = 1024 * 1024;

/// The helper path's own bound. A probe that never finishes is a failure, not an
/// open-ended wait.
pub const DEFAULT_HELPER_TIMEOUT: Duration = Duration::from_secs(60);

/// Diagnostics kept even when nothing else is captured: a transport failure has
/// to be classified, and `ssh` says very little.
const DIAGNOSTIC_CAPTURE: usize = 64 * 1024;

pub struct Request<'a> {
    pub host: &'a str,
    pub command: &'a str,
    pub cwd: Option<&'a str>,
    pub env: &'a [(String, String)],
    /// `None` means no deadline: an ordinary foreground run waits until the
    /// remote program finishes (EXEC-002).
    pub timeout: Option<Duration>,
    /// `None` means keep nothing in memory; `Some(0)` means keep everything.
    pub capture: Option<usize>,
    pub fresh: bool,
}

/// How a running child learns that the local invocation should stop, and which
/// signal answered. Two closures rather than one because the poll has to be
/// cheap and the reason has to be exact: `Run::cancelled` says *that* the run
/// was stopped, the signal says whether the process status is 130 or 143
/// (WIRE-004).
#[derive(Clone, Copy)]
pub struct Cancel<'a> {
    poll: &'a dyn Fn() -> bool,
    reason: &'a dyn Fn() -> Option<CancelSignal>,
}

fn never_requested() -> bool {
    false
}

fn never_cancelled() -> Option<CancelSignal> {
    None
}

impl<'a> Cancel<'a> {
    pub fn new(poll: &'a dyn Fn() -> bool, reason: &'a dyn Fn() -> Option<CancelSignal>) -> Self {
        Self { poll, reason }
    }

    fn signal(&self) -> Option<CancelSignal> {
        (self.reason)()
    }

    /// A run nobody may interrupt locally: the helper path, where the caller has
    /// no terminal to lose.
    pub fn none() -> Self {
        Self {
            poll: &never_requested,
            reason: &never_cancelled,
        }
    }
}

#[derive(Debug)]
pub enum ExecError {
    /// The run happened but cannot be described. Not a connectivity claim.
    Internal(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Internal(reason) => write!(f, "{reason}"),
        }
    }
}

/// Application-level validation, which runs *after* the CLI has rejected grammar
/// problems. The distinction is load-bearing: an empty `--command` is a usage
/// error, while whitespace-only content reaches here and is `CONFIG_INVALID`
/// (WIRE-006).
fn invalid(request: &Request<'_>) -> bool {
    request.command.trim().is_empty()
        || request
            .capture
            .is_some_and(|limit| limit > MAX_CAPTURE_BYTES)
        || request
            .env
            .iter()
            .any(|(key, _)| !shell::is_valid_env_key(key))
}

fn spec<'a>(
    command: &'a str,
    request: &'a Request<'_>,
    nonce: &'a InvocationToken,
) -> ExecSpec<'a> {
    ExecSpec {
        command,
        cwd: request.cwd,
        env: request.env,
        nonce,
        remote_state_dir: None,
    }
}

/// Runs a foreground command, forwarding its output as it is produced.
///
/// `cancellation` is polled while the child runs, and answers with the signal
/// that ended the local invocation: the reason distinguishes 130 from 143 and
/// must never be guessed after the fact.
pub fn execute_stream(
    client: &Client,
    request: &Request<'_>,
    forward_stdout: Option<Box<dyn Write>>,
    forward_stderr: Option<Box<dyn Write>>,
    stdin: StdinSource<'_>,
    cancellation: Cancel<'_>,
) -> Result<ExecOutcome, ExecError> {
    if invalid(request) {
        return Ok(ExecOutcome::not_started(
            PreExecFailure::Config,
            ProcessOutput::empty(),
            0,
        ));
    }
    let nonce = protocol::new_nonce().map_err(|error| ExecError::Internal(error.to_string()))?;
    let script = protocol::wrap_script(&protocol::build_script(
        &spec(request.command, request, &nonce),
        Separator::Nul,
    ));
    let mut stdout = CompletionStream::new(Separator::Nul, &nonce, forward_stdout, request.capture);
    let mut stderr = Tap::new(
        forward_stderr,
        request.capture.unwrap_or(DIAGNOSTIC_CAPTURE),
        Keep::Prefix,
    );
    let run = client
        .run_streams(
            &Streams {
                target: request.host,
                remote_command: &script,
                timeout: request.timeout,
                stdin,
                fresh: request.fresh,
                cancelled: cancellation.poll,
            },
            &mut stdout,
            &mut stderr,
        )
        .map_err(|error| ExecError::Internal(error.to_string()))?;
    let output = process_output(stdout.body(), stdout.total(), stderr.body(), stderr.total())?;
    let ended = decide(
        client,
        request,
        &nonce,
        &run,
        stdout.completion(),
        &output,
        cancellation,
    );
    realize(nonce, ended, output, run.duration)
}

/// Runs a command and reads both streams once it is over. Used by probes and
/// helpers that need the whole result, never by the `exec` CLI path.
pub fn execute_captured(
    client: &Client,
    command: &str,
    request: &Request<'_>,
    stdin: Option<&[u8]>,
) -> Result<ExecOutcome, ExecError> {
    if invalid(request) {
        return Ok(ExecOutcome::not_started(
            PreExecFailure::Config,
            ProcessOutput::empty(),
            0,
        ));
    }
    let nonce = protocol::new_nonce().map_err(|error| ExecError::Internal(error.to_string()))?;
    let script = protocol::wrap_script(&protocol::build_script(
        &spec(command, request, &nonce),
        Separator::Newline,
    ));
    let limit = request.capture.unwrap_or(0);
    let captured = client
        .run_captured(
            request.host,
            &script,
            request.timeout.unwrap_or(DEFAULT_HELPER_TIMEOUT),
            limit,
            stdin,
            request.fresh,
        )
        .map_err(|error| ExecError::Internal(error.to_string()))?;
    let (stdout_body, code) = match protocol::parse_marker(&captured.stdout, &nonce) {
        Some((body, code)) => (body.to_vec(), Some(code)),
        None => (captured.stdout.clone(), None),
    };
    // The counters describe the wrapper's own stdout, which carries the begin and
    // completion markers; the body does not. Subtracting what the parse removed
    // keeps stdout_bytes the size of the *command's* output, so a truncation flag
    // means "the command produced more than you were given".
    let removed = captured.stdout.len().saturating_sub(stdout_body.len()) as u64;
    let stdout_bytes = captured.stdout_bytes.saturating_sub(removed);
    let output = process_output(
        truncate(stdout_body, limit),
        stdout_bytes,
        truncate(captured.stderr, limit),
        captured.stderr_bytes,
    )?;
    let ended = decide(
        client,
        request,
        &nonce,
        &captured.run,
        code,
        &output,
        Cancel::none(),
    );
    realize(nonce, ended, output, captured.run.duration)
}
