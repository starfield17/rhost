//! CLI grammar, rendering and process status.
//!
//! The parser is hand-written because the grammar is a public contract: the
//! shape of an error must not depend on a flag library's help text or error
//! wording. Every failure carries an `error.code` an agent can branch on, and
//! `--json` never mixes prose into the one document on stdout (AGENTS.md §6).
//!
//! Parsing is total and side-effect free. Nothing here contacts a host or starts
//! a remote tool, so an invalid invocation cannot leave a remote side effect
//! behind (WIRE-006).
//!
//! The cut is by capability, not by kind: `grammar` and `commands` parse the
//! non-file commands, `files` owns everything about `fs` (grammar, running and
//! prose), `tunnel` owns everything about `tunnel`, `run` executes the rest,
//! `console` delivers, `render` and `usage` write. `parse_invocation` is the only
//! entry point the binary uses.

mod audit;
mod commands;
mod console;
mod files;
mod grammar;
mod render;
mod run;
mod session;
mod tunnel;
mod usage;

pub use grammar::parse_invocation;

use crate::fileops::runner::Stop;
use crate::output;
use crate::signals::Interrupt;
use crate::transport::openssh::Client;
use console::{Failure, Sink, warn};

/// Upper bound on a `--command-file`. A shell program this long is already
/// unusual; the bound exists so a wrong path cannot read a whole volume into
/// memory.
pub(crate) const MAX_COMMAND_FILE_BYTES: u64 = 64 * 1024;

/// The limit `--max-output-bytes` may name, spelled as the CLI states it.
pub(crate) const MAX_OUTPUT_LIMIT: i64 = 64 * 1024 * 1024;

/// A parsed invocation: what to do, and whether the caller expects one JSON
/// document on stdout.
pub struct Invocation {
    pub command: Command,
    pub json: bool,
}

/// Everything the grammar accepted, already validated as grammar.
pub enum Command {
    Version,
    /// Human help text for one page. Unreachable with `--json`, where help
    /// becomes a usage envelope: prose on stdout would break a decoder
    /// (WIRE-002).
    Help(Help),
    Hosts,
    Doctor {
        host: String,
        /// `--timeout` in nanoseconds. `0` means the default probe budget, and a
        /// negative value is a configuration error (see `parse_duration`).
        timeout_nanos: i64,
        fresh: bool,
    },
    ConnectionStatus {
        host: String,
    },
    ConnectionReset {
        host: String,
    },
    Exec(Exec),
    /// Every file operation: parsed, run and rendered in `files`.
    Fs(files::Fs),
    /// Every tunnel operation: parsed, run and rendered in `tunnel`.
    Tunnel(tunnel::Tunnel),
    /// Every session operation: parsed, run and rendered in `session`.
    Session(session::Session),
    /// A recognised operation this build refuses under its real schema operation
    /// name, because the v2 contract has no honest answer for it.
    Refused {
        operation: &'static str,
        message: String,
    },
    /// The local audit trail of remote operations.
    Audit {
        /// At most this many of the most recent entries; `0` means all of them.
        limit: usize,
        /// Empty means every host.
        host: String,
    },
    /// A usage error. `scope` selects which `*.usage` operation is reported.
    Usage {
        scope: Scope,
        message: String,
    },
}

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

/// The exact shell program, as named by the caller. Which of the two it was is
/// part of the error surface: a file is read locally, so its failures are
/// `CONFIG_INVALID` reported against `exec`, never a remote diagnostic.
pub enum CommandText {
    Inline(String),
    File(String),
}

pub struct Exec {
    pub host: String,
    pub program: CommandText,
    pub cwd: Option<String>,
    /// Raw `KEY=VALUE` operands, split and validated before anything runs.
    pub env: Vec<String>,
    /// `--timeout` in nanoseconds; `0` means no deadline, and a negative value
    /// is a configuration error rather than a parse error (EXEC-002).
    pub timeout_nanos: i64,
    /// `--max-output-bytes` as typed. `None` means the caller did not name a
    /// limit, which differs from naming zero (keep everything).
    pub max_output_bytes: Option<i64>,
    pub fresh: bool,
    pub stream: bool,
}

/// What the process should exit with, plus whether stdout itself was lost.
pub struct Delivery {
    pub status: u8,
    /// Set when stdout could not be written: reported on stderr and never
    /// retried, because the operation may already have completed (EXEC-010).
    pub delivery_failed: bool,
}
impl Scope {
    fn name(self) -> &'static str {
        match self {
            Self::Root => "rhost",
            Self::Session => "session",
            Self::Connection => "connection",
            Self::Tunnel => "tunnel",
            Self::Fs => "fs",
        }
    }
}

impl Invocation {
    /// Runs the parsed invocation against a transport client and delivers the
    /// result. Returns what the process should exit with.
    pub fn run(self, client: &Client, interrupt: &Interrupt) -> Delivery {
        let json = self.json;
        let mut sink = Sink::new();
        // One interruption source for every operation that starts a process, so
        // a signal the CLI can receive is a signal it can act on.
        let stop_requested = || interrupt.requested();
        let stop_signal = || interrupt.signal();
        let stop = Stop {
            requested: &stop_requested,
            signal: &stop_signal,
        };
        let status = match self.command {
            Command::Version => {
                if json {
                    sink.envelope(&output::version());
                } else {
                    run::version(&mut sink);
                }
                0
            }
            Command::Help(help) => {
                sink.text(&usage::help(help));
                0
            }
            Command::Hosts => run::hosts(&mut sink, json),
            Command::Doctor {
                host,
                timeout_nanos,
                fresh,
            } => run::doctor(&mut sink, client, &host, timeout_nanos, fresh, json),
            Command::ConnectionStatus { host } => {
                run::connection_status(&mut sink, client, &host, json)
            }
            Command::ConnectionReset { host } => {
                run::connection_reset(&mut sink, client, &host, json)
            }
            Command::Fs(command) => files::run(&mut sink, client, command, json, stop),
            Command::Tunnel(command) => tunnel::run(&mut sink, client, command, json),
            Command::Session(command) => session::run(&mut sink, client, command, json),
            Command::Refused { operation, message } => {
                Failure::new(operation, "", "USAGE_ERROR", message).deliver(&mut sink, json)
            }
            Command::Audit { limit, host } => audit::run(&mut sink, limit, &host, json),
            Command::Exec(command) => run::exec(&mut sink, client, &command, json, interrupt),
            Command::Usage { scope, message } => {
                if json {
                    sink.envelope(&match scope {
                        Scope::Root => output::usage(&message),
                        other => output::group_usage(other.name()),
                    });
                } else {
                    warn(&format!("rhost: USAGE_ERROR: {message}"));
                    if scope == Scope::Root && wants_help(&message) {
                        sink.text(&usage::help(Help::Root));
                    }
                }
                255
            }
        };
        sink.finish(status)
    }
}

/// Only the root's own "needs a subcommand" error is paired with the help text.
fn wants_help(message: &str) -> bool {
    message == "rhost needs a subcommand"
}
