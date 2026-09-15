//! The flag machinery and the whole-argv parser.
//!
//! Nothing here decides what an operation *means*: this module turns argv into
//! [`Command`](super::Command) values and reports the first thing the caller
//! mistyped. Every refusal it produces is a `USAGE_ERROR`.

use super::audit;
use super::commands;
use super::files;
use super::session;
use super::tunnel;
use super::{Command, Help, Invocation, Scope};

mod flags;
mod scan;

pub(crate) use flags::{
    FlagSpec, JSON, PLAIN_FLAGS, ROOT_FLAGS_ALL, doctor_flags, exec_flags, flags,
};
pub(crate) use scan::{Parsed, find_command, parse, parse_duration, wants_json};

/// Parses a whole argv. `--json` is recognised before anything can fail, because
/// a parse failure has to be delivered in the form the caller asked for.
pub fn parse_invocation(argv: &[String]) -> Invocation {
    let json = wants_json(argv);
    let Some(at) = find_command(argv) else {
        // No subcommand: the root parses its own flags and either explains
        // itself or refuses.
        return match parse(argv, ROOT_FLAGS_ALL) {
            Err(message) => Invocation {
                command: usage_error(Scope::Root, message),
                json,
            },
            Ok(parsed) if parsed.bool_flag("version") => Invocation {
                command: Command::Version,
                json,
            },
            Ok(parsed) if parsed.bool_flag("help") || !json && parsed.operands.is_empty() => {
                if json {
                    return Invocation {
                        command: help_or_error(Scope::Root, Help::Root, json),
                        json,
                    };
                }
                let _ = parsed;
                Invocation {
                    command: Command::Help(Help::Root),
                    json,
                }
            }
            Ok(_) => Invocation {
                command: usage_error(Scope::Root, "rhost needs a subcommand".to_string()),
                json,
            },
        };
    };
    let group = match argv.get(at) {
        Some(token) => token.clone(),
        None => {
            return Invocation {
                command: usage_error(Scope::Root, "rhost needs a subcommand".to_string()),
                json,
            };
        }
    };
    // The subcommand parses everything except its own name, in the original
    // order, so a flag written before the command works exactly as one written
    // after it.
    let rest: Vec<String> = argv
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != at)
        .map(|(_, token)| token.clone())
        .collect();
    let command = match group.as_str() {
        "exec" => commands::exec_command(&rest, json),
        "doctor" => commands::doctor_command(&rest, json),
        "hosts" => {
            commands::simple_command(&rest, Command::Hosts, "rhost hosts", Help::Hosts, json)
        }
        "version" => commands::simple_command(
            &rest,
            Command::Version,
            "rhost version",
            Help::Version,
            json,
        ),
        "connection" => commands::connection_command(&rest, json),
        "session" => session::command(&rest, json),
        "tunnel" => tunnel::command(&rest, json),
        "fs" => files::command(&rest, json),
        "audit" => audit::command(&rest, json),
        other => usage_error(
            Scope::Root,
            format!("unknown command {other} for \"rhost\""),
        ),
    };
    Invocation { command, json }
}

pub(crate) fn usage_error(scope: Scope, message: String) -> Command {
    Command::Usage { scope, message }
}

/// Requesting help with `--json` asks for two mutually exclusive things. The
/// envelope wins, because an agent that set `--json` cannot read prose.
pub(crate) fn help_or_error(scope: Scope, help: Help, json: bool) -> Command {
    if json {
        return usage_error(
            scope,
            format!(
                "--help prints prose; {} --json reports one document",
                scope.name()
            ),
        );
    }
    Command::Help(help)
}
