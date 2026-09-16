//! `exec` grammar: the flag table and the mapping from argv to one [`Exec`].

use super::Exec;
use crate::cli::{CommandText, Help, ParsedCommand, Scope, exec_flags, parse, parse_duration};

/// Parses `rhost exec`. The program is taken by flag so that exactly one shell
/// program crosses the boundary (EXEC-001); an explicit `--` therefore refuses a
/// trailing command rather than guessing which operand was meant.
pub fn command(argv: &[String], json: bool) -> ParsedCommand<Exec> {
    let parsed = match parse(argv, exec_flags()) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Root, message),
    };
    if parsed.bool_flag("help") {
        return ParsedCommand::help_or_error(Scope::Root, Help::Exec, json);
    }
    if parsed.dash {
        return ParsedCommand::usage(
            Scope::Root,
            "rhost exec does not accept a command after --; use --command or --command-file",
        );
    }
    if parsed.operands.len() != 1 {
        return ParsedCommand::usage(
            Scope::Root,
            if parsed.operands.is_empty() {
                "rhost exec requires <host>".to_string()
            } else {
                format!(
                    "rhost exec accepts 1 arg(s), received {}",
                    parsed.operands.len()
                )
            },
        );
    }
    let inline = parsed.occurrences("command") > 0;
    let file = parsed.occurrences("command-file") > 0;
    if inline == file {
        return ParsedCommand::usage(
            Scope::Root,
            "rhost exec requires exactly one of --command or --command-file",
        );
    }
    if inline && parsed.value("command") == Some("") {
        return ParsedCommand::usage(Scope::Root, "--command must not be empty");
    }
    if file && matches!(parsed.value("command-file"), Some("") | Some("-")) {
        return ParsedCommand::usage(
            Scope::Root,
            "--command-file requires a local file path, not stdin",
        );
    }
    if parsed.bool_flag("stream") && !json {
        return ParsedCommand::usage(Scope::Root, "--stream requires --json");
    }
    let timeout = match parsed.value("timeout") {
        Some(raw) => match parse_duration(raw) {
            Some(nanos) => nanos,
            None => {
                return ParsedCommand::usage(
                    Scope::Root,
                    format!("invalid duration for --timeout: {raw}"),
                );
            }
        },
        None => 0,
    };
    let max_output = match parsed.value("max-output-bytes") {
        Some(raw) => match raw.parse::<i64>() {
            Ok(value) => Some(value),
            Err(_) => {
                return ParsedCommand::usage(
                    Scope::Root,
                    format!("invalid value for --max-output-bytes: {raw}"),
                );
            }
        },
        None => None,
    };
    ParsedCommand::Run(Exec {
        host: parsed.operands[0].clone(),
        program: if file {
            CommandText::File(parsed.value("command-file").unwrap_or_default().to_string())
        } else {
            CommandText::Inline(parsed.value("command").unwrap_or_default().to_string())
        },
        cwd: parsed
            .value("cwd")
            .filter(|cwd| !cwd.is_empty())
            .map(str::to_string),
        env: parsed.all("env"),
        timeout_nanos: timeout,
        max_output_bytes: max_output,
        fresh: parsed.bool_flag("fresh"),
        stream: parsed.bool_flag("stream"),
    })
}
