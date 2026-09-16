//! Bounded output: source bytes counted, whatever the rendering drops.

use super::ExecError;
use crate::domain::{CapturedText, ProcessOutput};
use crate::shell;

pub(super) fn process_output(
    stdout: Vec<u8>,
    stdout_bytes: u64,
    stderr: Vec<u8>,
    stderr_bytes: u64,
) -> Result<ProcessOutput, ExecError> {
    Ok(ProcessOutput {
        stdout: capture(stdout, stdout_bytes)?,
        stderr: capture(stderr, stderr_bytes)?,
    })
}

/// Counts describe *source* bytes, so a rendering that replaces an invalid byte
/// never changes them, and a truncated capture still reports what was observed.
fn capture(body: Vec<u8>, bytes: u64) -> Result<CapturedText, ExecError> {
    let observed = bytes.max(body.len() as u64);
    CapturedText::from_bytes(body, observed).map_err(|error| ExecError::Internal(error.to_string()))
}

pub(super) fn truncate(body: Vec<u8>, limit: usize) -> Vec<u8> {
    if limit == 0 {
        return body;
    }
    shell::truncate_on_char_boundary(body, limit)
}
