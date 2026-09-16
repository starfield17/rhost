//! One helper submission, one refusal, one code.
//!
//! Every use-case submits its script through [`run_helper`] and looks its
//! refusal up here, so the same cause reports the same `error.code` whichever
//! operation hit it.

use super::shared::helper_budget;
use crate::remote::Error;
use crate::remote::{RemoteRun, run_remote, run_remote_partial};
use crate::session::{self, HelperFailure};
use crate::transport::Client;
use std::time::Duration;

/// One helper submission: the script runs through the ordinary exec path, and a
/// helper that could not report its own result at all is a session problem rather
/// than a caller mistake.
pub(super) fn run_helper(
    client: &Client,
    host: &str,
    script: &str,
    user_timeout: Duration,
    capture: usize,
) -> Result<String, Error> {
    let (code, stdout, stderr) = run_remote(
        client,
        host,
        script,
        None,
        helper_budget(user_timeout),
        capture,
    )?;
    if code != 0 {
        return Err(unhealthy(&format!(
            "session helper exited {code}{}",
            detail(&stderr)
        )));
    }
    Ok(stdout)
}

/// One helper submission that keeps whatever it printed whether or not it
/// completed. `create` needs the partial answer: the remote may have made a
/// session before the response was lost.
pub(super) fn run_helper_once(
    client: &Client,
    host: &str,
    script: &str,
    user_timeout: Duration,
    capture: usize,
) -> Result<RemoteRun, Error> {
    run_remote_partial(
        client,
        host,
        script,
        None,
        helper_budget(user_timeout),
        capture,
    )
}

pub(super) fn detail(stderr: &str) -> String {
    let first = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if first.is_empty() {
        String::new()
    } else {
        format!(": {first}")
    }
}

/// The helper's refusal, when it reported one. A code outside the closed set is
/// kept as an unknown failure rather than silently treated as success.
pub(super) fn helper_failure(stdout: &str) -> Option<HelperFailure> {
    session::field(stdout, "RHOST_ERR=").map(HelperFailure::from_code)
}

pub(super) fn session_error(failure: HelperFailure) -> Error {
    match failure {
        HelperFailure::NoSession | HelperFailure::SessionDied => Error::new(
            "SESSION_NOT_FOUND",
            if failure == HelperFailure::SessionDied {
                "session shell exited (the command likely ran `exit`)"
            } else {
                "no such session"
            },
        ),
        HelperFailure::Timeout => Error::new(
            "REMOTE_COMMAND_TIMEOUT",
            "command exceeded timeout in session",
        ),
        HelperFailure::Locked => Error::new(
            "SESSION_UNHEALTHY",
            "session is busy (another writer holds the lock)",
        )
        .retryable(),
        HelperFailure::InputFailed => {
            Error::new("SESSION_UNHEALTHY", "could not submit command input to session").retryable()
        }
        HelperFailure::Busy => Error::new(
            "SESSION_BUSY",
            "session foreground is not the managed shell; use session send/read, or session recover",
        )
        .retryable(),
        HelperFailure::UnknownForeground => {
            Error::new("SESSION_UNHEALTHY", "could not read the pane's foreground command")
                .retryable()
        }
        HelperFailure::NoTty => {
            Error::new("SESSION_UNHEALTHY", "could not open the pane's terminal").retryable()
        }
        HelperFailure::NotReady => Error::new(
            "SESSION_UNHEALTHY",
            "session shell did not become ready",
        )
        .retryable(),
        HelperFailure::NoTmux => Error::new(
            "REMOTE_DEPENDENCY_MISSING",
            "remote host is missing tmux",
        ),
        HelperFailure::NoFlock => Error::new(
            "REMOTE_DEPENDENCY_MISSING",
            "remote host is missing flock (util-linux)",
        ),
        HelperFailure::NameInUse => Error::new("CONFIG_INVALID", "session name is already in use"),
        HelperFailure::InvalidCwd => Error::new(
            "CONFIG_INVALID",
            "initial working directory is not accessible",
        ),
        HelperFailure::NewFailed => {
            Error::new("SESSION_UNHEALTHY", "could not create the tmux session").retryable()
        }
        HelperFailure::Protocol | HelperFailure::Unknown(_) => Error::new(
            "SESSION_UNHEALTHY",
            "session helper returned incomplete or invalid completion evidence",
        ),
    }
}

pub(super) fn unhealthy(message: &str) -> Error {
    Error::new("SESSION_UNHEALTHY", message).retryable()
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// The key names rhost accepts. Anything else is arbitrary text that would be
/// injected as a key name, so the set is a closed allowlist rather than a
/// passthrough.
pub(super) fn is_supported_key(key: &str) -> bool {
    matches!(
        key,
        "C-c"
            | "C-d"
            | "C-z"
            | "C-\\"
            | "C-u"
            | "C-l"
            | "C-a"
            | "C-e"
            | "C-w"
            | "Enter"
            | "Tab"
            | "BTab"
            | "Space"
            | "BSpace"
            | "Escape"
            | "Up"
            | "Down"
            | "Left"
            | "Right"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
    )
}
