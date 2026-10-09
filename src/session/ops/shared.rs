//! What every session use-case agrees on: how much of a helper's answer is kept,
//! the transport budget that outlives the helper, and the record's timestamp.

use std::time::Duration;

/// A helper's output is a few line-protocol fields, except exec's, which carries
/// the command's own terminal output inline. The helper caps that segment at
/// `session::EXEC_OUTPUT_LIMIT`; this local capture is the backstop and must hold
/// the base64 form of that bound plus the other fields, so a valid answer is
/// never truncated into a protocol failure. It is still a bound, so a runaway
/// helper cannot become an unbounded local allocation.
pub(super) const FIELD_CAPTURE: usize = 4 * 1024 * 1024;
pub(super) const OUTPUT_CAPTURE: usize = crate::remote::exec::MAX_CAPTURE_BYTES;

/// The helper's own worst case: a non-blocking lock, up to five seconds waiting
/// for the pane to go idle, then the caller's deadline. The transport has to
/// outlive that, or it would kill the helper before it could interrupt the
/// command and report whether the session survived.
pub(super) fn helper_budget(user: Duration) -> Duration {
    user + Duration::from_secs(2 + 5 + 10)
}

pub(super) fn now() -> String {
    // The remote host's own clock is the authority; this is the timestamp of the
    // record, and it only has to be honest about when rhost created it.
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    format!("unix:{seconds}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_capture_holds_the_largest_legitimate_exec_answer() {
        // base64 expands three bytes to four; the rest of the answer is a small,
        // fixed set of fields, so a kilobyte of headroom is generous.
        let encoded = crate::session::EXEC_OUTPUT_LIMIT.div_ceil(3) * 4;
        assert!(
            FIELD_CAPTURE > encoded + 1024,
            "FIELD_CAPTURE {FIELD_CAPTURE} must exceed base64({}) + overhead {encoded}",
            crate::session::EXEC_OUTPUT_LIMIT
        );
    }
}
