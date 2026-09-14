//! `create`: make one remote tmux session and return the record for it.

use super::errors::{helper_failure, run_helper, session_error, unhealthy, valid_name};
use super::lifecycle::Info;
use super::shared::{OUTPUT_CAPTURE, now};
use crate::app::Error;
use crate::app::remote::internal;
use crate::session::{self, Meta, Status};
use crate::transport::openssh::Client;
use std::time::Duration;

/// What a caller passes to `session create`.
pub struct CreateOptions<'a> {
    pub host: &'a str,
    /// Empty means "use the id as the name".
    pub name: &'a str,
    pub cwd: &'a str,
    pub shell: &'a str,
    pub timeout: Duration,
}

/// Creates a session and returns the record the remote host now owns.
pub fn create(client: &Client, options: &CreateOptions<'_>) -> Result<Info, Error> {
    let shell_name = if options.shell.is_empty() {
        session::DEFAULT_SHELL
    } else {
        options.shell
    };
    if shell_name != session::DEFAULT_SHELL {
        return Err(Error::new(
            "CONFIG_INVALID",
            "only --shell bash is supported",
        ));
    }
    if !options.name.is_empty() && !valid_name(options.name) {
        return Err(Error::new(
            "CONFIG_INVALID",
            "invalid session name (use letters, digits, _ . -)",
        ));
    }
    let id = session::new_id()
        .map_err(|error| internal(format!("could not generate session id: {error}")))?;
    let name = if options.name.is_empty() {
        id.clone()
    } else {
        options.name.to_string()
    };
    let meta = Meta::new(&id, &name, options.cwd, shell_name, &now());
    let script = session::create_script(&meta, "bash --noprofile --norc -i");
    let stdout = run_helper(
        client,
        options.host,
        &script,
        options.timeout,
        OUTPUT_CAPTURE,
    )?;
    if let Some(failure) = helper_failure(&stdout) {
        return Err(session_error(failure));
    }
    if !stdout.contains("RHOST_OK=created") {
        return Err(unhealthy("session was not created"));
    }
    Ok(Info {
        id,
        name,
        tmux_session: meta.tmux_session,
        created_at: meta.created_at,
        initial_cwd: meta.initial_cwd,
        shell: meta.shell,
        status: Status::Alive,
    })
}
