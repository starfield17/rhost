//! `doctor` grammar: one host, an optional probe budget, and `--fresh`.

use super::{DEFAULT_TIMEOUT_NANOS, Doctor};
use crate::cli::{Help, ParsedCommand, Scope, doctor_flags, parse, parse_duration};

pub fn command(argv: &[String], json: bool) -> ParsedCommand<Doctor> {
    let parsed = match parse(argv, doctor_flags()) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Root, message),
    };
    if parsed.bool_flag("help") {
        return ParsedCommand::help_or_error(Scope::Root, Help::Doctor, json);
    }
    if parsed.operands.len() != 1 {
        return ParsedCommand::usage(
            Scope::Root,
            if parsed.operands.is_empty() {
                "rhost doctor requires <host>".to_string()
            } else {
                format!(
                    "rhost doctor accepts 1 arg(s), received {}",
                    parsed.operands.len()
                )
            },
        );
    }
    let timeout_nanos = match parsed.value("timeout") {
        Some(raw) => match parse_duration(raw) {
            Some(nanos) => nanos,
            None => {
                return ParsedCommand::usage(
                    Scope::Root,
                    format!("invalid duration for --timeout: {raw}"),
                );
            }
        },
        None => DEFAULT_TIMEOUT_NANOS,
    };
    ParsedCommand::Run(Doctor {
        host: parsed.operands[0].clone(),
        timeout_nanos,
        fresh: parsed.bool_flag("fresh"),
    })
}
