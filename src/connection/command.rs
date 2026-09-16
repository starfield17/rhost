//! `connection` grammar: `status` and `reset` each take exactly one host.

use super::Connection;
use crate::cli::{Help, PLAIN_FLAGS, ParsedCommand, Scope, parse};

pub fn command(argv: &[String], json: bool) -> ParsedCommand<Connection> {
    let parsed = match parse(argv, PLAIN_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Connection, message),
    };
    let Some(leaf) = parsed.operand(0).map(str::to_string) else {
        return if parsed.bool_flag("help") {
            ParsedCommand::help_or_error(Scope::Connection, Help::Group(Scope::Connection), json)
        } else {
            ParsedCommand::usage(Scope::Connection, "rhost connection needs a subcommand")
        };
    };
    if parsed.bool_flag("help") {
        if json {
            return ParsedCommand::help_or_error(
                Scope::Connection,
                Help::Group(Scope::Connection),
                json,
            );
        }
        return ParsedCommand::HelpLeaf(Scope::Connection, leaf);
    }
    let build: fn(String) -> Connection = match leaf.as_str() {
        "status" => |host| Connection::Status { host },
        "reset" => |host| Connection::Reset { host },
        other => {
            return ParsedCommand::usage(
                Scope::Root,
                format!("unknown command {other} for \"rhost connection\""),
            );
        }
    };
    if parsed.operands.len() != 2 {
        return ParsedCommand::usage(
            Scope::Root,
            format!(
                "rhost connection {leaf} accepts 1 arg(s), received {}",
                parsed.operands.len().saturating_sub(1)
            ),
        );
    }
    match parsed.operand(1) {
        Some(host) => ParsedCommand::Run(build(host.to_string())),
        None => ParsedCommand::usage(
            Scope::Root,
            format!("rhost connection {leaf} requires <host>"),
        ),
    }
}
