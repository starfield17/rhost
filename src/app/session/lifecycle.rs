//! `lifecycle`: what sessions exist, and how one ends — cleanly, or by
//! interrupting what holds it.

use super::errors::{helper_failure, run_helper, session_error, unhealthy};
use super::shared::FIELD_CAPTURE;
use crate::app::Error;
use crate::session::{self, HelperFailure, ListEntry, Status};
use crate::transport::openssh::Client;
use std::time::Duration;

/// One session record as the caller sees it.
#[derive(Debug, Clone)]
pub struct Info {
    pub id: String,
    pub name: String,
    pub tmux_session: String,
    pub created_at: String,
    pub initial_cwd: String,
    pub shell: String,
    pub status: Status,
}

impl Info {
    fn from_entry(entry: &ListEntry) -> Self {
        Self {
            id: entry.meta.id.clone(),
            name: entry.meta.name.clone(),
            tmux_session: entry.meta.tmux_session.clone(),
            created_at: entry.meta.created_at.clone(),
            initial_cwd: entry.meta.initial_cwd.clone(),
            shell: entry.meta.shell.clone(),
            status: if entry.alive {
                Status::Alive
            } else {
                Status::Dead
            },
        }
    }
}
/// Every session this account has a record for, with each one's liveness as
/// remote tmux just reported it.
pub fn list(client: &Client, host: &str, timeout: Duration) -> Result<Vec<Info>, Error> {
    let stdout = run_helper(
        client,
        host,
        &session::list_script(),
        timeout,
        FIELD_CAPTURE,
    )?;
    match session::parse_list(&stdout) {
        Ok(entries) => Ok(entries.iter().map(Info::from_entry).collect()),
        Err(failure) => Err(session_error(failure)),
    }
}
/// The answer to "is this session usable again?", which is a different question
/// from "did the interrupt land".
#[derive(Debug, Clone)]
pub struct Recover {
    pub id: Option<String>,
    pub reference: String,
    pub preserved: bool,
    pub foreground: Option<String>,
    /// Present when the shell did not come back; the data above is still the
    /// answer, so it travels with the failure.
    pub failure: Option<Error>,
}

pub fn recover(
    client: &Client,
    host: &str,
    reference: &str,
    timeout: Duration,
) -> Result<Recover, Error> {
    let stdout = run_helper(
        client,
        host,
        &session::recover_script(reference, timeout),
        timeout,
        FIELD_CAPTURE,
    )?;
    let mut result = Recover {
        id: session::field(&stdout, "RHOST_ID=")
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        reference: reference.to_string(),
        preserved: false,
        foreground: None,
        failure: None,
    };
    if let Some(failure) = helper_failure(&stdout) {
        result.foreground = session::field(&stdout, "RHOST_FG=")
            .filter(|name| !name.is_empty())
            .map(str::to_string);
        if failure == HelperFailure::Busy {
            let what = result.foreground.as_deref().unwrap_or("another program");
            result.failure = Some(
                Error::new(
                    "SESSION_BUSY",
                    format!("session foreground is {what}, not the managed shell; use session send/read, or session recover"),
                )
                .retryable(),
            );
        } else {
            result.failure = Some(session_error(failure));
        }
        return Ok(result);
    }
    if result.id.is_none() || !stdout.contains("RHOST_OK=recovered") {
        result.preserved = false;
        result.failure = Some(unhealthy("session shell readiness was not proven"));
        return Ok(result);
    }
    result.preserved = true;
    Ok(result)
}

pub fn close(client: &Client, host: &str, reference: &str, timeout: Duration) -> Result<(), Error> {
    let stdout = run_helper(
        client,
        host,
        &session::close_script(reference),
        timeout,
        FIELD_CAPTURE,
    )?;
    if let Some(failure) = helper_failure(&stdout) {
        return Err(session_error(failure));
    }
    if !stdout.contains("RHOST_OK=closed") {
        return Err(unhealthy("session close was not confirmed"));
    }
    Ok(())
}
