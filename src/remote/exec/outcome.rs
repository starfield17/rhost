//! What a finished run amounts to, and the one place the invocation token is
//! spent.
//!
//! A run's evidence is decided here and nowhere else: completion is verified
//! against the token that submitted it, an interrupt keeps whatever evidence
//! exists, and a missing marker is reported as uncertainty rather than guessed
//! into a status.

use super::cleanup::stop_group;
use super::{Cancel, CancelSignal, Client, ExecError, Request};
use crate::domain::{
    CleanupEvidence, CompletionEvidence, ExecOutcome, ExitCode, Interruption, InvocationToken,
    PreExecFailure, ProcessOutput,
};
use crate::transport::{Run, RunFailure};
use std::time::Duration;

/// What a finished run amounts to, decided while the invocation token is still
/// unconsumed. The token is spent exactly once, in [`realize`].
pub(super) enum Ended {
    /// Completion evidence for this invocation, no local failure.
    Completed(i32),
    /// The local invocation was interrupted. Completion evidence, if any, is
    /// kept independent of the interruption (EXEC-008).
    Interrupted {
        code: Option<i32>,
        reason: Interruption,
        cleanup: CleanupEvidence,
    },
    /// Output could not be delivered. The remote side may have completed.
    Undeliverable {
        code: Option<i32>,
        cleanup: CleanupEvidence,
    },
    /// Nothing was submitted, and why.
    NotStarted(PreExecFailure),
    /// The command may or may not have run. Never a known success and never an
    /// assumed connectivity failure (EXEC-005).
    Unknown,
}

pub(super) fn decide(
    client: &Client,
    request: &Request<'_>,
    nonce: &InvocationToken,
    run: &Run,
    code: Option<i32>,
    output: &ProcessOutput,
    cancellation: Cancel<'_>,
) -> Ended {
    if code.is_none()
        && !run.timed_out
        && !run.cancelled
        && let Some(reason) = pre_submission(run)
    {
        return Ended::NotStarted(reason);
    }
    if matches!(run.failure, Some(RunFailure::Stream(_))) {
        return Ended::Undeliverable {
            code,
            // A finished foreground program's process group is not this call's
            // to destroy: detached work is out of scope (EXEC-007).
            cleanup: match code {
                Some(_) => CleanupEvidence::NotAttempted,
                None => stop_group(client, request, nonce),
            },
        };
    }
    if run.timed_out || run.cancelled {
        let reason = if run.timed_out {
            Interruption::Timeout
        } else {
            Interruption::Cancelled(cancellation.signal().unwrap_or(CancelSignal::Term))
        };
        return Ended::Interrupted {
            code,
            reason,
            cleanup: match code {
                Some(_) => CleanupEvidence::NotAttempted,
                None => stop_group(client, request, nonce),
            },
        };
    }
    if let Some(code) = code {
        // A completed foreground program can be followed by a local failure when
        // background work inherited the SSH pipe. The token-bound status stays
        // valid; it describes that program, never the work it launched.
        return Ended::Completed(code);
    }
    match classify(output, run.exit) {
        Verdict::NotStarted(reason) => Ended::NotStarted(reason),
        Verdict::Unknown => Ended::Unknown,
    }
}
/// A child that could not be started at all is a transport fact, and an unsafe
/// local state directory is a configuration fact. Both are known before the
/// remote boundary is crossed.
fn pre_submission(run: &Run) -> Option<PreExecFailure> {
    match run.failure {
        Some(RunFailure::Spawn(ref error))
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            Some(PreExecFailure::Config)
        }
        Some(RunFailure::Spawn(_)) => Some(PreExecFailure::Connection),
        _ => None,
    }
}

/// Spends the invocation token once. A completion that cannot be verified is not
/// downgraded to a guess: it is reported as no evidence at all.
pub(super) fn realize(
    nonce: InvocationToken,
    ended: Ended,
    output: ProcessOutput,
    duration: Duration,
) -> Result<ExecOutcome, ExecError> {
    let duration_ms = duration.as_millis().try_into().unwrap_or(u64::MAX);
    match ended {
        Ended::Completed(code) => match verified(nonce, code) {
            Some(completion) => Ok(ExecOutcome::completed(completion, output, duration_ms)),
            None => Ok(ExecOutcome::unknown(output, duration_ms)),
        },
        Ended::Interrupted {
            code,
            reason,
            cleanup,
        } => ExecOutcome::interrupted(
            evidence(nonce, code),
            reason,
            cleanup,
            output.clone(),
            duration_ms,
        )
        .map_err(|error| ExecError::Internal(error.to_string())),
        Ended::Undeliverable { code, cleanup } => Ok(ExecOutcome::output_failed(
            evidence(nonce, code),
            cleanup,
            output,
            duration_ms,
        )),
        Ended::NotStarted(reason) => Ok(ExecOutcome::not_started(reason, output, duration_ms)),
        Ended::Unknown => Ok(ExecOutcome::unknown(output, duration_ms)),
    }
}

fn evidence(nonce: InvocationToken, code: Option<i32>) -> CompletionEvidence {
    match code {
        Some(code) => match verified(nonce, code) {
            Some(completion) => CompletionEvidence::Completed(completion),
            None => CompletionEvidence::Missing,
        },
        None => {
            drop(nonce);
            CompletionEvidence::Missing
        }
    }
}

/// Binds an observed exit status to the invocation that submitted it. The token
/// is spent here, so one completion cannot be replayed into another call.
fn verified(nonce: InvocationToken, code: i32) -> Option<crate::domain::VerifiedCompletion> {
    let observed = nonce.as_str().to_string();
    let exit = ExitCode::new(code).ok()?;
    nonce.verify_completion(&observed, exit).ok()
}
enum Verdict {
    NotStarted(PreExecFailure),
    Unknown,
}

/// Maps a run whose completion evidence never appeared onto what it means. A
/// command that exited non-zero is *completed* evidence, so this only ever sees
/// runs with no completion at all.
fn classify(output: &ProcessOutput, exit: Option<i32>) -> Verdict {
    match crate::remote::pre_execution(&output.stderr.content(), exit) {
        Some(kind) => Verdict::NotStarted(kind),
        None => Verdict::Unknown,
    }
}
