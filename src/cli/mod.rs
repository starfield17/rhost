//! CLI primitives shared by every capability.
//!
//! This module knows nothing about what any capability *means*. It owns the
//! vocabulary a capability parses with and renders through: flag tables and the
//! argv scanner, the `--help`/usage shapes, the stdout [`Sink`], and the
//! [`Failure`] type that maps an `error.code` onto a process status.
//!
//! The parser is hand-written because the grammar is a public contract: the
//! shape of an error must not depend on a flag library's help text or error
//! wording. Every failure carries an `error.code` an agent can branch on, and
//! `--json` never mixes prose into the one document on stdout (AGENTS.md §6).
//!
//! Parsing is total and side-effect free. Nothing here contacts a host or starts
//! a remote tool, so an invalid invocation cannot leave a remote side effect
//! behind (WIRE-006).

mod console;
mod flags;
mod scan;

pub use console::{Failure, Sink, field, ok_missing, warn, yes};
pub use flags::{FlagSpec, JSON, PLAIN_FLAGS, ROOT_FLAGS_ALL, doctor_flags, exec_flags};
pub use scan::{Parsed, find_command, parse, parse_duration, wants_json};

/// Which part of the grammar an error belongs to. Every variant names a real
/// `operation` value in the schema's closed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Root,
    Session,
    Connection,
    Tunnel,
    Fs,
}

impl Scope {
    pub fn name(self) -> &'static str {
        match self {
            Self::Root => "rhost",
            Self::Session => "session",
            Self::Connection => "connection",
            Self::Tunnel => "tunnel",
            Self::Fs => "fs",
        }
    }
}

/// One page of human help: the root's usage, a group's subcommand list, or one
/// command with its own flags. Every command a caller can run has a page, so
/// `--help` can never show one command's flag set under another command's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Help {
    Root,
    Exec,
    Doctor,
    Hosts,
    Version,
    Audit,
    /// A group's overview: its subcommands, and the flags the group itself takes.
    Group(Scope),
    /// One leaf of a group, with that leaf's own flags.
    Leaf(Scope, &'static str),
}

/// A usage refusal: the scope whose `*.usage` operation is reported, plus the
/// diagnostic. The router turns it into the `USAGE_ERROR` envelope.
pub struct Rejection {
    pub scope: Scope,
    pub message: String,
}

impl Rejection {
    pub fn new(scope: Scope, message: impl Into<String>) -> Self {
        Self {
            scope,
            message: message.into(),
        }
    }
}

/// What a capability's parser hands back: a command to run, a help request, a
/// usage refusal, or a recognised operation this build refuses under its own
/// schema operation name (the v2 contract has no honest answer for `attach`).
pub enum ParsedCommand<T> {
    Run(T),
    Help(Help),
    /// A group leaf whose page or group overview is chosen by the dispatcher,
    /// which owns the prose tables. The leaf name is whatever the caller typed,
    /// so an unknown one falls back to the group overview.
    HelpLeaf(Scope, String),
    Reject(Rejection),
    Refused {
        operation: &'static str,
        message: String,
    },
}

impl<T> ParsedCommand<T> {
    /// Requesting help with `--json` asks for two mutually exclusive things. The
    /// envelope wins, because an agent that set `--json` cannot read prose.
    pub fn help_or_error(scope: Scope, help: Help, json: bool) -> Self {
        if json {
            return Self::Reject(Rejection::new(
                scope,
                format!(
                    "--help prints prose; {} --json reports one document",
                    scope.name()
                ),
            ));
        }
        Self::Help(help)
    }

    pub fn usage(scope: Scope, message: impl Into<String>) -> Self {
        Self::Reject(Rejection::new(scope, message))
    }
}

/// The exact shell program, as named by the caller. Which of the two it was is
/// part of the error surface: a file is read locally, so its failures are
/// `CONFIG_INVALID` reported against `exec`, never a remote diagnostic.
pub enum CommandText {
    Inline(String),
    File(String),
}

/// What the process should exit with, plus whether stdout itself was lost.
pub struct Delivery {
    pub status: u8,
    /// Set when stdout could not be written: reported on stderr and never
    /// retried, because the operation may already have completed (EXEC-010).
    pub delivery_failed: bool,
}
