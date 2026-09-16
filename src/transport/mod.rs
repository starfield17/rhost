//! Remote capabilities: the OpenSSH transport and its execution protocol.
//!
//! rhost orchestrates the system OpenSSH client; it never reimplements SSH,
//! authentication or host-key policy (AGENTS.md §5).
//!
//! This module is the boundary. `openssh` (the child and its argv), `process`
//! (the local loop that owns one child) and `protocol` (the wrapper script and
//! its completion marker) are private: a neighbor reaches the capability through
//! the surface below, so changing a wrapper detail does not require knowing who
//! imported it. `main.rs` is a separate crate and cannot see `pub(crate)`, hence
//! the plain `pub use`.

mod openssh;
mod process;
mod protocol;

pub use openssh::{Client, Config, ConnectionStatus, MasterStatus, Reset, Streams};
pub use process::{Capture, Keep, Recorder, Run, RunFailure, Spec, StdinSource, Stream, Tap, run};
pub use protocol::{
    CompletionStream, DEFAULT_REMOTE_STATE_DIR, ExecSpec, Separator, build_script, kill_command,
    new_nonce, parse_marker, wrap_script,
};
