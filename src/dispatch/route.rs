//! The whole-argv parser and the subcommand inventory.
//!
//! The root owns only the flags it can interpret before it knows the
//! subcommand: `--json`, `--version`, `--help`. Everything else is handed to a
//! capability's parser unchanged, in the original order, so a flag written
//! before the command works exactly as one written after it.

use super::Command;
use super::commands;
use crate::cli::{Help, ParsedCommand, Rejection, Scope, find_command, parse, wants_json};

/// Parses a whole argv. `--json` is recognised before anything can fail, because
/// a parse failure has to be delivered in the form the caller asked for.
pub fn parse_invocation(argv: &[String]) -> super::Invocation {
    let json = wants_json(argv);
    let Some(at) = find_command(argv) else {
        return super::Invocation {
            command: root(argv, json),
            json,
        };
    };
    let group = match argv.get(at) {
        Some(token) => token.clone(),
        None => {
            return super::Invocation {
                command: Command::Usage(Rejection::new(Scope::Root, "rhost needs a subcommand")),
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
        "exec" => wrap(crate::exec::command(&rest, json)),
        "doctor" => wrap(crate::doctor::command(&rest, json)),
        "hosts" => wrap(crate::hosts::command(&rest, json)),
        "version" => commands::version_command(&rest, json),
        "connection" => wrap(crate::connection::command(&rest, json)),
        "session" => wrap(crate::session::command(&rest, json)),
        "tunnel" => wrap(crate::tunnel::command(&rest, json)),
        "fs" => wrap(crate::files::command(&rest, json)),
        "audit" => wrap(crate::audit::command(&rest, json)),
        other => Command::Usage(Rejection::new(
            Scope::Root,
            format!("unknown command {other} for \"rhost\""),
        )),
    };
    super::Invocation { command, json }
}

/// The root flags on their own: no subcommand, so this either explains itself,
/// prints the version, or refuses.
fn root(argv: &[String], json: bool) -> Command {
    match parse(argv, crate::cli::ROOT_FLAGS_ALL) {
        Err(message) => Command::Usage(Rejection::new(Scope::Root, message)),
        Ok(parsed) if parsed.bool_flag("version") => Command::Version,
        Ok(parsed) if parsed.bool_flag("help") || !json && parsed.operands.is_empty() => {
            if json {
                match ParsedCommand::<()>::help_or_error(Scope::Root, Help::Root, json) {
                    ParsedCommand::Reject(rejection) => Command::Usage(rejection),
                    ParsedCommand::Help(help) => Command::Help(help),
                    _ => unreachable!("help_or_error only returns Help or Reject"),
                }
            } else {
                Command::Help(Help::Root)
            }
        }
        Ok(_) => Command::Usage(Rejection::new(Scope::Root, "rhost needs a subcommand")),
    }
}

/// Lifts a capability's parse result into one dispatch command, preserving the
/// help/refusal distinction the capability reported.
fn wrap<T>(parsed: ParsedCommand<T>) -> Command
where
    T: Into<Command>,
{
    match parsed {
        ParsedCommand::Run(value) => value.into(),
        ParsedCommand::Help(help) => Command::Help(help),
        ParsedCommand::HelpLeaf(scope, leaf) => Command::HelpLeaf(scope, leaf),
        ParsedCommand::Reject(rejection) => Command::Usage(rejection),
        ParsedCommand::Refused { operation, message } => Command::Refused { operation, message },
    }
}
