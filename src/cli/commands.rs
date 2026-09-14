//! Per-command grammar for everything except `fs`.
//!
//! Each command's flags are a compile-time table, so the flag set a command
//! accepts is a fact of the source rather than of a runtime registration. `fs`
//! keeps its grammar beside its execution, in `files`.

use super::grammar::{
    PLAIN_FLAGS, doctor_flags, exec_flags, flags, help_or_error, parse, parse_duration, usage_error,
};
use super::{Command, CommandText, Exec, Scope};

/// `hosts` and `version` take no operands and no flags beyond `--json`/`--help`.
pub(crate) fn simple_command(argv: &[String], command: Command, path: &str, json: bool) -> Command {
    let parsed = match parse(argv, PLAIN_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Root, message),
    };
    if parsed.bool_flag("help") {
        return help_or_error(Scope::Root, json);
    }
    if !parsed.operands.is_empty() || parsed.dash {
        return usage_error(
            Scope::Root,
            format!(
                "{path} accepts 0 arg(s), received {}",
                parsed.operands.len()
            ),
        );
    }
    command
}

pub(crate) fn connection_command(argv: &[String], json: bool) -> Command {
    let parsed = match parse(argv, flags(Scope::Connection)) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Connection, message),
    };
    if parsed.bool_flag("help") {
        return help_or_error(Scope::Connection, json);
    }
    let Some(leaf) = parsed.operand(0).map(str::to_string) else {
        return usage_error(
            Scope::Connection,
            "rhost connection needs a subcommand".to_string(),
        );
    };
    let command: fn(String) -> Command = match leaf.as_str() {
        "status" => |host| Command::ConnectionStatus { host },
        "reset" => |host| Command::ConnectionReset { host },
        other => {
            return usage_error(
                Scope::Root,
                format!("unknown command {other} for \"rhost connection\""),
            );
        }
    };
    if parsed.operands.len() != 2 {
        return usage_error(
            Scope::Root,
            format!(
                "rhost connection {leaf} accepts 1 arg(s), received {}",
                parsed.operands.len().saturating_sub(1)
            ),
        );
    }
    match parsed.operand(1) {
        Some(host) => command(host.to_string()),
        None => usage_error(
            Scope::Root,
            format!("rhost connection {leaf} requires <host>"),
        ),
    }
}

pub(crate) fn doctor_command(argv: &[String], json: bool) -> Command {
    let parsed = match parse(argv, doctor_flags()) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Root, message),
    };
    if parsed.bool_flag("help") {
        return help_or_error(Scope::Root, json);
    }
    if parsed.operands.len() != 1 {
        return usage_error(
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
                return usage_error(
                    Scope::Root,
                    format!("invalid duration for --timeout: {raw}"),
                );
            }
        },
        None => DOCTOR_DEFAULT_TIMEOUT_NANOS,
    };
    Command::Doctor {
        host: parsed.operands[0].clone(),
        timeout_nanos,
        fresh: parsed.bool_flag("fresh"),
    }
}

/// The probe budget a caller who said nothing gets. A probe that never finishes
/// is a failure, not an open-ended wait.
pub(crate) const DOCTOR_DEFAULT_TIMEOUT_NANOS: i64 = 60_000_000_000;

pub(crate) fn exec_command(argv: &[String], json: bool) -> Command {
    let parsed = match parse(argv, exec_flags()) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Root, message),
    };
    if parsed.bool_flag("help") {
        return help_or_error(Scope::Root, json);
    }
    // An explicit `--` cannot carry a command: exec takes its program by flag so
    // that exactly one shell program crosses the boundary (EXEC-001).
    if parsed.dash {
        return usage_error(
            Scope::Root,
            "rhost exec does not accept a command after --; use --command or --command-file"
                .to_string(),
        );
    }
    if parsed.operands.len() != 1 {
        return usage_error(
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
        return usage_error(
            Scope::Root,
            "rhost exec requires exactly one of --command or --command-file".to_string(),
        );
    }
    if inline && parsed.value("command") == Some("") {
        return usage_error(Scope::Root, "--command must not be empty".to_string());
    }
    if file && matches!(parsed.value("command-file"), Some("") | Some("-")) {
        return usage_error(
            Scope::Root,
            "--command-file requires a local file path, not stdin".to_string(),
        );
    }
    if parsed.bool_flag("stream") && !json {
        return usage_error(Scope::Root, "--stream requires --json".to_string());
    }
    let timeout = match parsed.value("timeout") {
        Some(raw) => match parse_duration(raw) {
            Some(nanos) => nanos,
            None => {
                return usage_error(
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
                return usage_error(
                    Scope::Root,
                    format!("invalid value for --max-output-bytes: {raw}"),
                );
            }
        },
        None => None,
    };
    Command::Exec(Exec {
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
