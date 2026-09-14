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

/// One flag the grammar knows.
#[derive(Clone, Copy)]
pub(crate) struct FlagSpec {
    name: &'static str,
    short: Option<char>,
    valued: bool,
    /// Reject a second occurrence. Only the two flags that carry a shell program
    /// are unique: silently letting the last `--cwd` win is worse than refusing,
    /// while a repeated boolean is harmless.
    unique: bool,
    /// The one line `--help` prints for this flag. It lives beside the flag so
    /// the help renderer can only describe flags the table holds, and a flag
    /// cannot be added without the line a human reads.
    help: &'static str,
}

impl FlagSpec {
    pub(crate) const fn long(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            short: None,
            valued: false,
            unique: false,
            help,
        }
    }
    pub(crate) const fn value(name: &'static str, unique: bool, help: &'static str) -> Self {
        Self {
            name,
            short: None,
            valued: true,
            unique,
            help,
        }
    }
    pub(crate) const fn value_short(
        name: &'static str,
        short: char,
        unique: bool,
        help: &'static str,
    ) -> Self {
        Self {
            name,
            short: Some(short),
            valued: true,
            unique,
            help,
        }
    }
    /// What the help renderer reads: the spelling callers type, whether it takes
    /// a value, and the line that describes it.
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }
    pub(crate) fn short(&self) -> Option<char> {
        self.short
    }
    pub(crate) fn valued(&self) -> bool {
        self.valued
    }
    pub(crate) fn help(&self) -> &'static str {
        self.help
    }
}

pub(crate) struct Flag {
    name: &'static str,
    value: String,
}

pub(crate) struct Parsed {
    pub(crate) operands: Vec<String>,
    pub(crate) flags: Vec<Flag>,
    /// A `--` separator was seen. Its presence matters: `exec` refuses operands
    /// after it rather than guessing which one was meant as a command.
    pub(crate) dash: bool,
}

impl Parsed {
    pub(crate) fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|flag| flag.name == name)
            .map(|flag| flag.value.as_str())
    }

    pub(crate) fn occurrences(&self, name: &str) -> usize {
        self.flags.iter().filter(|flag| flag.name == name).count()
    }

    pub(crate) fn all(&self, name: &str) -> Vec<String> {
        self.flags
            .iter()
            .filter(|flag| flag.name == name)
            .map(|flag| flag.value.clone())
            .collect()
    }

    pub(crate) fn bool_flag(&self, name: &str) -> bool {
        self.value(name) == Some("true")
    }

    pub(crate) fn operand(&self, index: usize) -> Option<&str> {
        self.operands.get(index).map(String::as_str)
    }
}

pub(crate) const JSON: FlagSpec =
    FlagSpec::long("json", "one machine-readable JSON document on stdout");

pub(crate) static ROOT_FLAGS_ALL: &[FlagSpec] = &[
    JSON,
    FlagSpec::long("version", "print the version and exit"),
];

pub(crate) static EXEC_FLAGS_ALL: &[FlagSpec] = &[
    JSON,
    FlagSpec::value_short("command", 'c', true, "the shell program to run"),
    FlagSpec::value("command-file", true, "read the program from a local file"),
    FlagSpec::value("cwd", false, "working directory on the remote host"),
    FlagSpec::value("timeout", false, "execution deadline; 0 waits forever"),
    FlagSpec::value(
        "max-output-bytes",
        false,
        "captured bytes per stream; 0 = all",
    ),
    FlagSpec::value("env", false, "environment variable KEY=VALUE (repeatable)"),
    FlagSpec::long("fresh", "use an independent SSH connection"),
    FlagSpec::long("stream", "mirror live output to stderr; JSON stays"),
];

pub(crate) static PLAIN_FLAGS: &[FlagSpec] = &[JSON];
pub(crate) static DOCTOR_FLAGS_ALL: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false, "probe budget; 0 uses the default"),
    FlagSpec::long("fresh", "use an independent SSH connection"),
];

/// The flags every scope accepts, plus its own. `--json` is a root persistent
/// flag: it may be written on any command, in any position before `--`.
pub(crate) fn flags(scope: Scope) -> &'static [FlagSpec] {
    // Each table repeats `--json` rather than concatenating at run time, so the
    // flag set stays a compile-time fact.
    match scope {
        Scope::Root => ROOT_FLAGS_ALL,
        Scope::Session | Scope::Connection | Scope::Tunnel | Scope::Fs => PLAIN_FLAGS,
    }
}

pub(crate) fn exec_flags() -> &'static [FlagSpec] {
    EXEC_FLAGS_ALL
}

pub(crate) fn doctor_flags() -> &'static [FlagSpec] {
    DOCTOR_FLAGS_ALL
}
/// Splits argv into flags and operands.
///
/// A value flag takes the next argument whether or not it looks like a flag,
/// which is what the original flag parser does; `--cwd --json` therefore sets
/// cwd to the literal `--json` instead of being guessed at. Errors come back in
/// left-to-right order, so the first thing a caller mistyped is the first thing
/// reported.
pub(crate) fn parse(argv: &[String], specs: &[FlagSpec]) -> Result<Parsed, String> {
    let mut operands: Vec<String> = Vec::new();
    let mut flags: Vec<Flag> = Vec::new();
    let mut dash = false;
    let mut index = 0;
    while index < argv.len() {
        let Some(token) = argv.get(index) else { break };
        index += 1;
        if dash {
            operands.push(token.clone());
            continue;
        }
        if token == "--" {
            dash = true;
            continue;
        }
        if let Some(body) = token.strip_prefix("--") {
            let (name, explicit) = match body.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (body, None),
            };
            let Some(spec) = specs.iter().find(|spec| spec.name == name) else {
                if name == "help" {
                    flags.push(Flag {
                        name: "help",
                        value: "true".into(),
                    });
                    continue;
                }
                return Err(format!("unknown flag --{name}"));
            };
            let value = if spec.valued {
                match explicit {
                    Some(value) => value,
                    None => match argv.get(index) {
                        Some(next) => {
                            index += 1;
                            next.clone()
                        }
                        None => return Err(format!("flag --{name} needs an argument")),
                    },
                }
            } else {
                match explicit {
                    None => "true".to_string(),
                    Some(raw) if raw == "true" || raw == "false" => raw,
                    Some(raw) => {
                        return Err(format!("invalid boolean value {raw} for --{name}"));
                    }
                }
            };
            if spec.unique && flags.iter().any(|flag| flag.name == spec.name) {
                return Err(format!("--{name} may be provided exactly once"));
            }
            flags.push(Flag {
                name: spec.name,
                value,
            });
            continue;
        }
        if token.len() > 1 && token.starts_with('-') {
            let body = &token[1..];
            let mut characters = body.chars();
            let Some(character) = characters.next() else {
                continue;
            };
            let spec = match specs.iter().find(|spec| spec.short == Some(character)) {
                Some(spec) => *spec,
                None => {
                    if character == 'h' {
                        flags.push(Flag {
                            name: "help",
                            value: "true".into(),
                        });
                        continue;
                    }
                    return Err(format!("unknown flag -{character}"));
                }
            };
            let remainder: String = characters.collect();
            let value = if spec.valued {
                if remainder.is_empty() {
                    match argv.get(index) {
                        Some(next) => {
                            index += 1;
                            next.clone()
                        }
                        None => return Err(format!("flag --{} needs an argument", spec.name)),
                    }
                } else {
                    remainder
                        .strip_prefix('=')
                        .unwrap_or(&remainder)
                        .to_string()
                }
            } else if remainder.is_empty() {
                "true".to_string()
            } else {
                return Err(format!("unknown flag -{remainder}"));
            };
            if spec.unique && flags.iter().any(|flag| flag.name == spec.name) {
                return Err(format!("--{} may be provided exactly once", spec.name));
            }
            flags.push(Flag {
                name: spec.name,
                value,
            });
            continue;
        }
        operands.push(token.clone());
    }
    Ok(Parsed {
        operands,
        flags,
        dash,
    })
}

/// Locates the subcommand the way the original CLI did: a token that is neither
/// a flag nor the value of a flag is the command name, and everything after `--`
/// is an operand rather than a candidate command.
///
/// The scan must not validate flags — at this point the only flags whose shape
/// rhost knows are the root's own booleans, so any other `--flag` is assumed to
/// take a value and its argument is skipped. Validating here would reject a
/// subcommand's flags before the subcommand ever got to explain them.
pub(crate) fn find_command(argv: &[String]) -> Option<usize> {
    let mut skip_next = false;
    for (index, token) in argv.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if token == "--" {
            // Everything after the separator is an operand, never a command name.
            return None;
        }
        if let Some(body) = token.strip_prefix("--") {
            let (name, has_value) = match body.split_once('=') {
                Some((name, _)) => (name, true),
                None => (body, false),
            };
            if !has_value && !root_boolean(name) {
                skip_next = true;
            }
            continue;
        }
        let short = token.strip_prefix('-').filter(|body| !body.is_empty());
        if let Some(name) = short {
            if name.len() == 1 && !name.contains('=') && !root_boolean(name) {
                skip_next = true;
            }
            continue;
        }
        if token.is_empty() {
            continue; // an empty operand is not a command name either
        }
        return Some(index);
    }
    None
}

/// The root's own boolean flags: `-h`/`--help`, `--json`, `--version`. Anything
/// else the root sees belongs to a subcommand and swallows the next token.
fn root_boolean(name: &str) -> bool {
    matches!(name, "json" | "version" | "help") || name == "h"
}

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

/// `--json` may appear anywhere before the `--` separator, including in a
/// position where the parse itself fails.
pub(crate) fn wants_json(argv: &[String]) -> bool {
    argv.iter()
        .take_while(|token| token.as_str() != "--")
        .any(|token| token == "--json" || token == "--json=true")
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

pub(crate) fn parse_duration(text: &str) -> Option<i64> {
    let body = match text.strip_prefix('-') {
        Some(rest) => {
            let magnitude = parse_duration(rest)?;
            return magnitude.checked_neg();
        }
        None => text.strip_prefix('+').unwrap_or(text),
    };
    if body == "0" {
        return Some(0);
    }
    let mut total: i64 = 0;
    let mut rest = body;
    while !rest.is_empty() {
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if digits.is_empty() || digits == "." {
            return None;
        }
        let number: f64 = digits.parse().ok()?;
        rest = &rest[digits.len()..];
        let (unit, remainder) = match rest.chars().next() {
            Some('n') if rest.starts_with("ns") => ("ns", &rest[2..]),
            Some('u') if rest.starts_with("us") => ("us", &rest[2..]),
            Some('µ') if rest.starts_with("µs") => ("us", &rest[3..]),
            Some('m') if rest.starts_with("ms") => ("ms", &rest[2..]),
            Some('s') => ("s", &rest[1..]),
            Some('m') => ("m", &rest[1..]),
            Some('h') => ("h", &rest[1..]),
            _ => return None,
        };
        let scale: i64 = match unit {
            "ns" => 1,
            "us" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60_000_000_000,
            "h" => 3_600_000_000_000,
            _ => return None,
        };
        let add = (number * scale as f64) as i64;
        total = total.checked_add(add)?;
        rest = remainder;
    }
    Some(total)
}
