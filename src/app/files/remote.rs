//! The file editing and transfer helpers.
//!
//! The path out to a host is [`crate::app::remote`], shared with sessions so a
//! transport failure is classified exactly once. What lives here is what is
//! specific to files: the helper protocol, its refusal codes, and the capability
//! probe a transfer needs before it starts.

use super::super::Error;
use super::super::exec;
pub(crate) use super::super::remote::{internal, run_remote};
use super::PROBE_TIMEOUT;
use crate::fileops as helper_protocol;
use crate::fileops::Refusal;
use crate::transport::{Client, MasterStatus};
use serde_json::Value;
use std::time::Duration;

/// A refusal that happened before anything ran: the caller can fix the
/// invocation rather than retry it.
pub(crate) fn validation(refusal: Refusal) -> Error {
    match refusal {
        Refusal::InvalidPath(message) => Error::new("CONFIG_INVALID", message),
        Refusal::Rejected(message) => Error::new("SYNC_REJECTED", message),
    }
}

pub(crate) fn first_line(text: &str) -> String {
    text.replace("\r\n", "\n")
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Runs the embedded helper for one request. Its codes are trusted because they
/// are a closed list, never because it said so.
pub(crate) fn helper(
    client: &Client,
    host: &str,
    request: &Value,
    timeout: Duration,
    max_bytes: usize,
) -> Result<Value, Error> {
    let payload = serde_json::to_vec(request).map_err(|error| internal(error.to_string()))?;
    let (code, stdout, stderr) = run_remote(
        client,
        host,
        &helper_protocol::command(),
        Some(&payload),
        timeout,
        helper_protocol::output_budget(max_bytes),
    )?;
    if code == 127 {
        return Err(Error::new(
            "REMOTE_DEPENDENCY_MISSING",
            "this operation needs remote python3; rhost reports it, it does not install it",
        ));
    }
    if code != 0 {
        return Err(internal(format!(
            "remote file helper exited {code}: {}",
            first_line(&stderr)
        )));
    }
    let answer: Value = serde_json::from_str(stdout.trim()).map_err(|_| {
        internal(format!(
            "remote file helper returned no JSON: {}",
            first_line(&stderr)
        ))
    })?;
    match helper_protocol::refusal(&answer) {
        Some((code, message)) => match known_helper_code(&code) {
            Some(known) => Err(Error::new(known, message)),
            None => Err(internal(format!("unknown remote file error {code}"))),
        },
        None => Ok(answer),
    }
}

/// The refusal codes the helper may use. Anything else is the remote inventing
/// an error, which is `INTERNAL` here rather than a code an agent might branch
/// on.
pub(crate) fn known_helper_code(code: &str) -> Option<&'static str> {
    const CODES: [&str; 12] = [
        "CONFIG_INVALID",
        "FILE_CONFLICT",
        "HASH_REQUIRED",
        "FILE_NOT_FOUND",
        "FILE_TOO_LARGE",
        "INVALID_PATCH",
        "INVALID_TARGET",
        "INVALID_TEXT",
        "SYNC_REJECTED",
        "REMOTE_DEPENDENCY_MISSING",
        "INTERNAL",
        "USAGE_ERROR",
    ];
    CODES
        .into_iter()
        .find(|known| known.eq_ignore_ascii_case(code))
}

/// The capability question `fs sync` cannot work without. Asking first keeps
/// "the remote has no rsync" a capability answer instead of a tool failure the
/// caller has to decode from rsync's exit status.
pub(crate) fn remote_has_rsync(client: &Client, host: &str) -> Result<(), Error> {
    let (code, _, stderr) = run_remote(
        client,
        host,
        "sh -c 'command -v rsync >/dev/null 2>&1'",
        None,
        PROBE_TIMEOUT,
        exec::DEFAULT_JSON_CAPTURE,
    )?;
    dependency_verdict(code)
        .map_err(|error| Error::new(error.0, format!("{}{}", error.1, first_line(&stderr))))
}

/// The one place a capability probe's status becomes an answer. `command -v`
/// returns zero when it finds the command and a shell-specific non-zero status
/// when it does not (ordinary `sh` uses 1). Transport uncertainty has already
/// been classified by `run_remote`, so every completed non-zero answer here is
/// the capability fact this probe asked for.
pub(crate) fn dependency_verdict(exit: u8) -> Result<(), (&'static str, &'static str)> {
    match exit {
        0 => Ok(()),
        _ => Err((
            "REMOTE_DEPENDENCY_MISSING",
            "the remote host has no rsync, which this operation needs; fs put and fs get work without it",
        )),
    }
}

pub(crate) fn master_alive(client: &Client, host: &str) -> bool {
    client.connection_status(host).master_status == MasterStatus::Alive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capability_probe_distinguishes_absence_from_failure() {
        assert!(dependency_verdict(0).is_ok());
        let code = |exit| dependency_verdict(exit).err().map(|error| error.0);
        assert_eq!(code(127), Some("REMOTE_DEPENDENCY_MISSING"));
        assert_eq!(code(1), Some("REMOTE_DEPENDENCY_MISSING"));
        assert_eq!(code(255), Some("REMOTE_DEPENDENCY_MISSING"));
    }

    #[test]
    fn ordinary_sh_uses_a_nonzero_capability_answer_for_absence() {
        let status = std::process::Command::new("sh")
            .args([
                "-c",
                "command -v rhost-deliberately-missing-probe >/dev/null 2>&1",
            ])
            .status()
            .unwrap_or_else(|error| panic!("run shell capability probe: {error}"));
        let exit = status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .unwrap_or_else(|| panic!("shell capability probe had no ordinary exit"));
        assert_ne!(exit, 0);
        assert_eq!(
            dependency_verdict(exit).err().map(|error| error.0),
            Some("REMOTE_DEPENDENCY_MISSING")
        );
    }
}
