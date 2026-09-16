//! The composition root: parse, route, and deliver.
//!
//! This module is deliberately thin. It owns the whole-argv grammar — the root
//! flags, the subcommand inventory, and the help/usage prose — and hands each
//! capability its own argv slice. It may depend on every capability, because
//! wiring them together is its only job; it holds no capability's behavior.
//!
//! Parsing is total and side-effect free. Nothing here contacts a host or
//! starts a remote tool, so an invalid invocation cannot leave a remote side
//! effect behind (WIRE-006). `parse_invocation` is the only entry point the
//! binary uses.

mod commands;
mod route;
mod run;
mod usage;

pub use route::parse_invocation;

use crate::cli::{Delivery, Failure, Help, Rejection, Scope, Sink, warn};
use crate::signals::Interrupt;
use crate::transport::Client;

/// A parsed invocation: what to do, and whether the caller expects one JSON
/// document on stdout.
pub struct Invocation {
    pub command: Command,
    pub json: bool,
}

/// Everything the grammar accepted, already validated as grammar. The
/// capability variants carry that capability's own parsed request; the rest are
/// the commands the composition root owns directly (version/help/usage).
pub enum Command {
    Version,
    Help(Help),
    /// A group leaf whose page or group overview the dispatcher resolves, since
    /// it owns the prose tables.
    HelpLeaf(Scope, String),
    Hosts(crate::hosts::Hosts),
    Doctor(crate::doctor::Doctor),
    Connection(crate::connection::Connection),
    Exec(crate::exec::Exec),
    Fs(crate::files::Fs),
    Tunnel(crate::tunnel::Command),
    Session(crate::session::Session),
    Audit(crate::audit::Audit),
    /// A recognised operation this build refuses under its real schema operation
    /// name, because the v2 contract has no honest answer for it.
    Refused {
        operation: &'static str,
        message: String,
    },
    /// A usage error. `scope` selects which `*.usage` operation is reported.
    Usage(Rejection),
}

macro_rules! command_from {
    ($ty:ty, $variant:ident) => {
        impl From<$ty> for Command {
            fn from(value: $ty) -> Self {
                Command::$variant(value)
            }
        }
    };
}

command_from!(crate::hosts::Hosts, Hosts);
command_from!(crate::doctor::Doctor, Doctor);
command_from!(crate::connection::Connection, Connection);
command_from!(crate::exec::Exec, Exec);
command_from!(crate::files::Fs, Fs);
command_from!(crate::tunnel::Command, Tunnel);
command_from!(crate::session::Session, Session);
command_from!(crate::audit::Audit, Audit);

impl Invocation {
    /// Runs the parsed invocation against a transport client and delivers the
    /// result. Returns what the process should exit with.
    pub fn run(self, client: &Client, interrupt: &Interrupt) -> Delivery {
        let json = self.json;
        let mut sink = Sink::new();
        let status = run::dispatch(&mut sink, client, self.command, json, interrupt);
        sink.finish(status)
    }
}

/// Delivers a bare usage refusal: the same `USAGE_ERROR` envelope an agent
/// branches on, with help prose only for the root's own "needs a subcommand".
pub(crate) fn deliver_usage(sink: &mut Sink, scope: Scope, message: &str, json: bool) -> u8 {
    if json {
        use crate::wire;
        sink.envelope(&match scope {
            Scope::Root => wire::usage(message),
            other => wire::group_usage(other.name()),
        });
    } else {
        warn(&format!("rhost: USAGE_ERROR: {message}"));
        if scope == Scope::Root && message == "rhost needs a subcommand" {
            sink.text(&usage::help(Help::Root));
        }
    }
    255
}

/// A refusal with no operation data, delivered by an operation that recognised
/// its own command but refused it (`session attach`).
pub(crate) fn deliver_refused(
    sink: &mut Sink,
    operation: &'static str,
    message: &str,
    json: bool,
) -> u8 {
    Failure::new(operation, "", "USAGE_ERROR", message).deliver(sink, json)
}
