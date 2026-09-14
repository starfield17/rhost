//! File transfer rules: what may cross the boundary, and how it is described.
//!
//! Two mechanisms, both borrowed from tools the user already has:
//!
//! * `scp` moves one file in either direction;
//! * `rsync` syncs a directory tree, with the incremental behaviour and explicit
//!   deletion rules the contract needs.
//!
//! Neither one resolves a host: the target is passed to the tool verbatim, so
//! OpenSSH stays the source of truth for alias, user, port, key and host-key
//! policy (AGENTS.md §5). The rules here are pure, so every refusal is testable
//! without a machine; only [`runner`] touches a process.
//!
//! Cut by what the rules are about: `args` judges the two operands of a
//! single-file copy and builds scp's argv, `sync` judges a directory sync and
//! builds rsync's, and `changes` reads rsync's itemized plan back. This file
//! owns the refusal every one of them reports.

mod args;
mod changes;
pub mod remote;
pub mod runner;
mod sync;

pub use args::{
    local_arg, remote_spec, scp_args, split_remote_spec, validate_remote_path,
    validate_transfer_paths,
};
pub use changes::{
    Action, CHANGE_MARKER, Change, parse_changes, path_base, path_needs_quoting,
    remote_parent_command,
};
pub use sync::{SyncPlan, SyncRequest, pull_args, push_args, reject_sync_target};

/// A refusal decided before anything runs. The distinction is the error code the
/// caller sees: a path mistake is theirs to fix (`CONFIG_INVALID`), while a
/// refused request is a deliberate safety decision (`SYNC_REJECTED`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    InvalidPath(String),
    Rejected(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath(message) | Self::Rejected(message) => f.write_str(message),
        }
    }
}

pub(super) fn invalid(message: impl Into<String>) -> Refusal {
    Refusal::InvalidPath(message.into())
}

pub(super) fn reject(message: impl Into<String>) -> Refusal {
    Refusal::Rejected(message.into())
}
