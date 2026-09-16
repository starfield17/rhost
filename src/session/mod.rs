//! Persistent sessions: a remote tmux session rhost can come back to.
//!
//! A session is owned entirely by remote tmux and remote files, so it survives
//! the CLI process, an SSH disconnect and the client machine rebooting (AGENTS.md
//! §4). rhost holds no live PTY and no map of what it created: it finds sessions
//! by reading the remote state directory, and the id printed by `create` is the
//! canonical identity every later call is resolved against.
//!
//! On the remote host each session owns
//!
//! ```text
//! $RHOST_REMOTE_STATE/sessions/<id>/
//!     meta.json   discovery metadata (authoritative)
//!     pty.log     raw pane output, appended by `tmux pipe-pane`
//!     lock        flock target serialising writers
//! ```
//!
//! The pane's shell is `bash --noprofile --norc -i` with an injected OSC 133
//! integration, so rhost gets a real command boundary and exit status without an
//! in-band sentinel. Terminal echo is off, which is what keeps captured output
//! clean; nothing here supports `attach`, which the v2 schema reports as a usage
//! error.

pub mod command;
mod dto;
mod ops;
pub mod remote;
mod render;
mod run;

pub use command::{Session, command};
pub use dto::{
    session_closed, session_create_failure, session_created, session_exec, session_exec_failure,
    session_exec_status, session_failure, session_read, session_recover, session_sent, sessions,
};
pub use ops::{
    CreateOptions, Created, CreationStatus, ExecOptions, Info, Recover, SendOptions, close, create,
    exec, list, read, recover, send,
};
pub use remote::protocol::{
    ExecResult, HelperFailure, ListEntry, ReadResult, field, parse_exec, parse_list, parse_read,
};
pub use run::run;

pub(crate) use remote::scripts::{
    SendKind, close as close_script, create as create_script, exec as exec_script,
    list as list_script, read as read_script, recover as recover_script, send as send_script,
};

use serde::{Deserialize, Serialize};

/// The interactive shell a managed pane runs. The OSC 133 integration needs bash
/// (or zsh), and the backend standardises on bash.
pub const DEFAULT_SHELL: &str = "bash";

/// The namespaced tmux session name for a session id.
pub fn tmux_name(id: &str) -> String {
    format!("rhost_s_{id}")
}

/// One session's discovery metadata. The remote helper writes it; every later
/// call reads it back over the same connection it was created on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub tmux_session: String,
    pub created_at: String,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub initial_cwd: String,
    #[serde(default)]
    pub shell: String,
}

impl Meta {
    pub fn new(id: &str, name: &str, cwd: &str, shell_name: &str, created_at: &str) -> Self {
        Self {
            schema_version: 1,
            id: id.to_string(),
            name: name.to_string(),
            tmux_session: tmux_name(id),
            created_at: created_at.to_string(),
            created_by: "rhost".to_string(),
            initial_cwd: cwd.to_string(),
            shell: shell_name.to_string(),
        }
    }
}

/// Whether the tmux session behind a record is still there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Alive,
    Dead,
    /// The record exists and its liveness could not be read.
    Unknown,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Dead => "dead",
            Self::Unknown => "unknown",
        }
    }
}

/// A session id: `s_` plus 48 bits of hex. Short enough to type, wide enough that
/// two sessions cannot collide by accident.
pub(crate) fn new_id() -> std::io::Result<String> {
    Ok(format!("s_{}", crate::random::hex(6)?))
}

/// A per-submission token. Completion evidence is only accepted for the
/// invocation that carries this token, so ordinary output cannot forge it
/// (SESSION-006).
pub(crate) fn new_token() -> std::io::Result<String> {
    crate::random::hex(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_tokens_are_hex_of_the_documented_width() {
        let id = new_id().unwrap_or_else(|error| panic!("{error}"));
        assert!(id.starts_with("s_") && id.len() == 14, "{id}");
        assert!(
            id[2..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "{id}"
        );
        assert_eq!(new_token().unwrap_or_default().len(), 32);
        assert_eq!(tmux_name("s_ab"), "rhost_s_s_ab");
    }

    #[test]
    fn metadata_round_trips_through_the_shape_the_helper_writes() {
        let meta = Meta::new(
            "s_ab",
            "work",
            "/tmp",
            DEFAULT_SHELL,
            "2026-01-01T00:00:00Z",
        );
        let text = serde_json::to_string(&meta).unwrap_or_default();
        let back: Meta = serde_json::from_str(&text).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(back.id, "s_ab");
        assert_eq!(back.tmux_session, "rhost_s_s_ab");
        assert_eq!(back.initial_cwd, "/tmp");
        // A record written without a cwd still reads: the field is optional.
        let minimal: Meta = serde_json::from_str(r#"{"schema_version":1,"id":"s_x","name":"n","tmux_session":"t","created_at":"now","shell":"bash"}"#)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(minimal.initial_cwd, "");
    }
}
