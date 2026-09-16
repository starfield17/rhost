//! The root-owned commands: `version`, and the shared shape for a command that
//! takes no operands beyond `--json`/`--help`.

use super::Command;
use crate::cli::{Help, PLAIN_FLAGS, ParsedCommand, Scope, parse};

/// `rhost version`.
pub(crate) fn version_command(argv: &[String], json: bool) -> Command {
    simple(argv, Help::Version, "rhost version", json)
}

/// A command with no operands and no flags beyond the shared ones. Anything the
/// caller adds is a usage error, named with the exact path they typed.
fn simple(argv: &[String], help: Help, path: &str, json: bool) -> Command {
    let parsed = match parse(argv, PLAIN_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return Command::Usage(crate::cli::Rejection::new(Scope::Root, message)),
    };
    if parsed.bool_flag("help") {
        return match ParsedCommand::<()>::help_or_error(Scope::Root, help, json) {
            ParsedCommand::Help(help) => Command::Help(help),
            ParsedCommand::Reject(rejection) => Command::Usage(rejection),
            _ => unreachable!("help_or_error only returns Help or Reject"),
        };
    }
    if !parsed.operands.is_empty() || parsed.dash {
        return Command::Usage(crate::cli::Rejection::new(
            Scope::Root,
            format!(
                "{path} accepts 0 arg(s), received {}",
                parsed.operands.len()
            ),
        ));
    }
    Command::Version
}
