//! The `tunnel` command surface: grammar, running and the mapping of a tunnel
//! fault onto the error taxonomy.
//!
//! The three subcommands are thin on purpose. Every decision about what a forward
//! may bind, and whether one is alive, belongs to `crate::tunnel` and to OpenSSH;
//! what this adds is the agent-facing contract — an id to keep, a status to
//! branch on, and one document per call (AGENTS.md §6).

use super::audit::Timer;
use super::console::{Failure, Sink};
use super::grammar::{FlagSpec, JSON, PLAIN_FLAGS, help_or_error, parse, usage_error};
use super::usage::leaf_or_group;
use super::{Command, Help, Scope};
use crate::output;
use crate::transport::Client;
use crate::tunnel::{self, Fault};

/// Every tunnel operation the parser can hand to [`run`].
pub enum Tunnel {
    Open {
        host: String,
        kind: String,
        listen: String,
        destination: String,
        expose: bool,
    },
    List,
    Close {
        id: String,
    },
}

/// The flags `open` takes. The group accepts them so the leaf can be found, and
/// every other leaf then parses the same argv again with its own table, which is
/// what refuses a flag that belongs to a sibling operation.
static TUNNEL_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("kind", false, "forward kind (local or remote)"),
    FlagSpec::value("listen", false, "address to listen on"),
    FlagSpec::value("destination", false, "forward destination host:port"),
    FlagSpec::long("allow-exposure", "permit binding a non-loopback address"),
];

/// The flags each leaf accepts, so its `--help` page and its parse read the same
/// table. `list` and `close` take operands only.
pub(crate) fn leaf_flags(leaf: &str) -> &'static [FlagSpec] {
    if leaf == "open" {
        TUNNEL_FLAGS
    } else {
        PLAIN_FLAGS
    }
}

/// Defaults that match the reference: the common case is a local forward out of
/// loopback, and a caller who wants something else says so.
const DEFAULT_KIND: &str = "local";
const DEFAULT_LISTEN: &str = "localhost:8080";

pub(crate) fn command(argv: &[String], json: bool) -> Command {
    let scanned = match parse(argv, TUNNEL_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Tunnel, message),
    };
    let leaf = scanned.operand(0).map(str::to_string);
    if scanned.bool_flag("help") {
        return help_or_error(
            Scope::Tunnel,
            match &leaf {
                Some(leaf) => leaf_or_group(Scope::Tunnel, leaf),
                None => Help::Group(Scope::Tunnel),
            },
            json,
        );
    }
    let Some(leaf) = leaf else {
        return usage_error(Scope::Tunnel, "rhost tunnel needs a subcommand".to_string());
    };
    let parsed = match parse(argv, leaf_flags(&leaf)) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Tunnel, message),
    };
    let operands = &parsed.operands[1.min(parsed.operands.len())..];
    match leaf.as_str() {
        "open" => {
            if let Err(message) = exact(&leaf, operands, 1, "<host>") {
                return usage_error(Scope::Tunnel, message);
            }
            let flag =
                |name: &str, default: &str| parsed.value(name).unwrap_or(default).to_string();
            Command::Tunnel(Tunnel::Open {
                host: operands[0].clone(),
                kind: flag("kind", DEFAULT_KIND),
                listen: flag("listen", DEFAULT_LISTEN),
                destination: flag("destination", ""),
                expose: parsed.bool_flag("allow-exposure"),
            })
        }
        "list" => {
            if let Err(message) = exact(&leaf, operands, 0, "") {
                return usage_error(Scope::Tunnel, message);
            }
            Command::Tunnel(Tunnel::List)
        }
        "close" => {
            if let Err(message) = exact(&leaf, operands, 1, "<id>") {
                return usage_error(Scope::Tunnel, message);
            }
            Command::Tunnel(Tunnel::Close {
                id: operands[0].clone(),
            })
        }
        other => usage_error(
            Scope::Tunnel,
            format!("unknown command {other} for \"rhost tunnel\""),
        ),
    }
}

fn exact(leaf: &str, operands: &[String], count: usize, names: &str) -> Result<(), String> {
    if operands.len() == count {
        return Ok(());
    }
    let wanted = if names.is_empty() {
        String::new()
    } else {
        format!(" {names}")
    };
    Err(format!(
        "rhost tunnel {leaf} accepts {count} arg(s){wanted}, received {}",
        operands.len()
    ))
}

/// Runs one tunnel operation, delivering the envelope or the human view, and
/// mapping a fault exactly once so a code never has to be repeated.
pub(crate) fn run(sink: &mut Sink, client: &Client, command: Tunnel, json: bool) -> u8 {
    match command {
        Tunnel::Open {
            host,
            kind,
            listen,
            destination,
            expose,
        } => {
            let timer = Timer::start("tunnel.open", &host);
            let request = match tunnel::Request::new(&host, &kind, &listen, &destination, expose) {
                Ok(request) => request,
                Err(fault) => {
                    let failure = failure("tunnel.open", &host, fault);
                    timer.failed(failure.code());
                    return failure.deliver(sink, json);
                }
            };
            match tunnel::open(client, &request) {
                Ok(opened) => {
                    timer.succeeded("", &format!("{kind} {listen} -> {destination}"), None);
                    if json {
                        sink.envelope(&output::tunnel("tunnel.open", &host, &opened));
                    } else {
                        super::render::tunnel(sink, &opened);
                    }
                    0
                }
                Err(fault) => {
                    let failure = failure("tunnel.open", &host, fault);
                    timer.failed(failure.code());
                    failure.deliver(sink, json)
                }
            }
        }
        Tunnel::List => match tunnel::list(client) {
            Ok(rows) => {
                if json {
                    sink.envelope(&output::tunnels(&rows));
                } else {
                    super::render::tunnels(sink, &rows);
                }
                0
            }
            Err(fault) => failure("tunnel.list", "", fault).deliver(sink, json),
        },
        Tunnel::Close { id } => {
            let timer = Timer::start("tunnel.close", "");
            match tunnel::close(client, &id) {
                Ok(closed) => {
                    timer.succeeded("", &closed, None);
                    if json {
                        sink.envelope(&output::tunnel_closed(&closed));
                    } else {
                        super::console::warn(&format!("closed {closed}"));
                    }
                    0
                }
                Err(fault) => {
                    let failure = failure("tunnel.close", "", fault);
                    timer.failed(failure.code());
                    failure.deliver(sink, json)
                }
            }
        }
    }
}

/// A missing record is not the same problem as a refused forward: an agent that
/// wants to clean up has to be able to tell "already gone" from "could not be
/// stopped", and only the transport-shaped failure is worth retrying.
fn failure(operation: &'static str, host: &str, fault: Fault) -> Failure {
    match fault {
        Fault::Invalid(message) => Failure::new(operation, host, "CONFIG_INVALID", message),
        Fault::NotFound(message) => Failure::new(operation, host, "TUNNEL_NOT_FOUND", message),
        Fault::Transport(message) => {
            Failure::new(operation, host, "TUNNEL_FAILED", message).retryable()
        }
    }
}
