//! `io`: inject input into a session's pane, and read its log back.

use super::errors::{helper_failure, is_supported_key, run_helper, session_error, unhealthy};
use super::shared::FIELD_CAPTURE;
use crate::app::Error;
use crate::session::{self, SendKind};
use crate::shell;
use crate::transport::openssh::Client;
use std::time::Duration;

/// What a caller passes to `session send`.
pub struct SendOptions<'a> {
    pub host: &'a str,
    pub session: &'a str,
    /// Exactly one of these is set.
    pub data: Option<&'a str>,
    pub key: Option<&'a str>,
    pub enter: bool,
}

/// Injects raw input or one control key. There is no completion boundary: this is
/// the path for a program that owns the pane.
pub fn send(client: &Client, options: &SendOptions<'_>, timeout: Duration) -> Result<(), Error> {
    // An empty `--data` is not a payload: the reference refuses it rather than
    // pasting nothing and calling that a send.
    let data = options.data.filter(|value| !value.is_empty());
    let key = options.key.filter(|value| !value.is_empty());
    let script = match (data, key) {
        (Some(data), None) => session::send_script(
            options.session,
            if options.enter {
                SendKind::DataEnter
            } else {
                SendKind::Data
            },
            data,
        ),
        (None, Some(key)) => {
            if options.enter {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    "provide --data [--enter] or exactly one --key",
                ));
            }
            if !is_supported_key(key) {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    format!("unsupported key {key}"),
                ));
            }
            session::send_script(options.session, SendKind::Key, key)
        }
        _ => {
            return Err(Error::new(
                "CONFIG_INVALID",
                "provide --data [--enter] or exactly one --key",
            ));
        }
    };
    let stdout = run_helper(client, options.host, &script, timeout, FIELD_CAPTURE)?;
    if let Some(failure) = helper_failure(&stdout) {
        return Err(session_error(failure));
    }
    if !stdout.contains("RHOST_OK=sent") {
        return Err(unhealthy("injected input was not confirmed"));
    }
    Ok(())
}

/// One page of a session's output log, with the cursor that follows it.
#[derive(Debug, Clone)]
pub struct Read {
    pub id: String,
    pub reference: String,
    pub from: u64,
    pub next: u64,
    pub size: u64,
    pub content: String,
}

impl Read {
    pub fn more(&self) -> bool {
        self.next < self.size
    }
}

pub fn read(
    client: &Client,
    host: &str,
    reference: &str,
    since: u64,
    timeout: Duration,
) -> Result<Read, Error> {
    let stdout = run_helper(
        client,
        host,
        &session::read_script(reference, since, 0),
        timeout,
        FIELD_CAPTURE,
    )?;
    let result = session::parse_read(&stdout).map_err(session_error)?;
    // A page cut at the byte limit can land mid-rune: hold the partial trailing
    // bytes back and move the cursor with them, so the next read re-delivers them
    // once complete rather than emitting a replacement character rhost invented.
    let mut data = result.data;
    let mut next = result.next;
    if next < result.size {
        let held = shell::incomplete_utf8_suffix(&data);
        if held > 0 {
            data.truncate(data.len() - held);
            next -= held as u64;
        }
    }
    Ok(Read {
        id: result.id,
        reference: reference.to_string(),
        from: result.from,
        next,
        size: result.size,
        content: shell::strip_ansi(&String::from_utf8_lossy(&data)),
    })
}
