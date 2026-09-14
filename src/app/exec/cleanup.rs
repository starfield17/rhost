//! The remote stop attempt, and what it is allowed to claim.

use super::{Client, DIAGNOSTIC_CAPTURE, KILL_TIMEOUT, Request};
use crate::domain::{CleanupEvidence, InvocationToken};
use crate::transport::protocol;

/// Asks the remote host to stop the process group recorded for this submission
/// and reports only what the attempt proved (EXEC-006, EXEC-007). Confirmation
/// never authorizes a retry, and never implies side effects were undone.
pub(super) fn stop_group(
    client: &Client,
    request: &Request<'_>,
    nonce: &InvocationToken,
) -> CleanupEvidence {
    let kill = protocol::wrap_script(&protocol::kill_command(nonce));
    match client.run_captured(
        request.host,
        &kill,
        KILL_TIMEOUT,
        DIAGNOSTIC_CAPTURE,
        None,
        request.fresh,
    ) {
        Ok(result)
            if !result.run.timed_out
                && !result.run.cancelled
                && String::from_utf8_lossy(&result.stdout).contains("cleanup-confirmed") =>
        {
            CleanupEvidence::ConfirmedStopped
        }
        _ => CleanupEvidence::Unconfirmed,
    }
}
