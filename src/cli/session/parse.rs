//! `session` grammar: the flag tables, the per-leaf budgets and the mapping from
//! argv to one [`Session`].

use crate::cli::grammar::{
    FlagSpec, JSON, PLAIN_FLAGS, Parsed, help_or_error, parse, parse_duration, usage_error,
};
use crate::cli::usage::leaf_or_group;
use crate::cli::{Command, Help, Scope};
use crate::session;
use std::time::Duration;

/// Every session operation the parser can hand to [`run`].
pub enum Session {
    Create {
        host: String,
        name: String,
        cwd: String,
        shell: String,
        timeout_nanos: i64,
    },
    List {
        host: String,
        timeout_nanos: i64,
    },
    Exec {
        host: String,
        session: String,
        command: String,
        timeout_nanos: i64,
    },
    Send {
        host: String,
        session: String,
        data: Option<String>,
        key: Option<String>,
        enter: bool,
    },
    Read {
        host: String,
        session: String,
        since: u64,
        timeout_nanos: i64,
    },
    Recover {
        host: String,
        session: String,
        timeout_nanos: i64,
    },
    Close {
        host: String,
        session: String,
    },
}

static CREATE_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("name", false, "session name; empty allocates one"),
    FlagSpec::value(
        "cwd",
        false,
        "remote working directory; resolved to an absolute path",
    ),
    FlagSpec::value("shell", false, "remote login shell (bash only)"),
    FlagSpec::value(
        "timeout",
        false,
        "operation deadline; 0 uses the 60-second default",
    ),
];
static LIST_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "operation deadline; 0 uses the default budget",
    ),
];
static EXEC_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("command", true, "shell program to run in the session"),
    FlagSpec::value(
        "timeout",
        false,
        "operation deadline; 0 uses the default budget",
    ),
];
static SEND_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("data", false, "text to paste into the session"),
    FlagSpec::value("key", false, "control key to send, e.g. C-c"),
    FlagSpec::long("enter", "press Enter after the data"),
];
static READ_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "since",
        false,
        "byte offset to resume from; 0 reads the start",
    ),
    FlagSpec::value(
        "timeout",
        false,
        "operation deadline; 0 uses the default budget",
    ),
];
static RECOVER_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "operation deadline; 0 uses the default budget",
    ),
];

/// The flags the group knows so a leaf can be found, exactly as for `fs`: a flag
/// belonging to a sibling leaf is then refused by that leaf's own table. The help
/// text is unused here — this table only locates the leaf.
static ANY_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("name", false, ""),
    FlagSpec::value("cwd", false, ""),
    FlagSpec::value("shell", false, ""),
    FlagSpec::value("command", false, ""),
    FlagSpec::value("data", false, ""),
    FlagSpec::value("key", false, ""),
    FlagSpec::value("since", false, ""),
    FlagSpec::value("timeout", false, ""),
    FlagSpec::long("enter", ""),
];

/// Each operation's own budget. A create waits for a shell to come up, while a
/// list or a close is one short round trip; a caller who says nothing gets the
/// reference's default rather than "no deadline at all".
pub(super) const CREATE_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const LIST_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const EXEC_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const SEND_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const RECOVER_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn command(argv: &[String], json: bool) -> Command {
    let scanned = match parse(argv, ANY_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Session, message),
    };
    let leaf = scanned.operand(0).map(str::to_string);
    if scanned.bool_flag("help") {
        return help_or_error(
            Scope::Session,
            match &leaf {
                Some(leaf) => leaf_or_group(Scope::Session, leaf),
                None => Help::Group(Scope::Session),
            },
            json,
        );
    }
    let Some(leaf) = leaf else {
        return usage_error(
            Scope::Session,
            "rhost session needs a subcommand".to_string(),
        );
    };
    let parsed = match parse(argv, flags(&leaf)) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Session, message),
    };
    let operands = &parsed.operands[1.min(parsed.operands.len())..];
    let value = |name: &str| parsed.value(name).map(str::to_string);
    let exact = |count: usize, names: &str| -> Result<(), String> {
        if operands.len() == count {
            return Ok(());
        }
        Err(format!(
            "rhost session {leaf} accepts {count} arg(s) {names}, received {}",
            operands.len()
        ))
    };
    match leaf.as_str() {
        "create" => {
            if let Err(message) = exact(1, "<host>") {
                return usage_error(Scope::Session, message);
            }
            let shell = value("shell").unwrap_or_else(|| session::DEFAULT_SHELL.to_string());
            Command::Session(Session::Create {
                host: operands[0].clone(),
                name: value("name").unwrap_or_default(),
                cwd: value("cwd").unwrap_or_default(),
                shell,
                timeout_nanos: match timeout(&parsed, CREATE_TIMEOUT) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Session, message),
                },
            })
        }
        "list" => {
            if let Err(message) = exact(1, "<host>") {
                return usage_error(Scope::Session, message);
            }
            Command::Session(Session::List {
                host: operands[0].clone(),
                timeout_nanos: match timeout(&parsed, LIST_TIMEOUT) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Session, message),
                },
            })
        }
        "exec" => {
            if let Err(message) = exact(2, "<host> <session>") {
                return usage_error(Scope::Session, message);
            }
            if parsed.dash {
                return usage_error(
                    Scope::Session,
                    "rhost session exec does not accept a command after --; use --command"
                        .to_string(),
                );
            }
            let command = match value("command") {
                Some(command) if !command.is_empty() => command,
                _ => {
                    return usage_error(
                        Scope::Session,
                        "rhost session exec requires --command <string>".to_string(),
                    );
                }
            };
            Command::Session(Session::Exec {
                host: operands[0].clone(),
                session: operands[1].clone(),
                command,
                timeout_nanos: match timeout(&parsed, EXEC_TIMEOUT) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Session, message),
                },
            })
        }
        "send" => {
            if let Err(message) = exact(2, "<host> <session>") {
                return usage_error(Scope::Session, message);
            }
            Command::Session(Session::Send {
                host: operands[0].clone(),
                session: operands[1].clone(),
                data: value("data"),
                key: value("key"),
                enter: parsed.bool_flag("enter"),
            })
        }
        "read" => {
            if let Err(message) = exact(2, "<host> <session>") {
                return usage_error(Scope::Session, message);
            }
            let since = match value("since") {
                None => 0,
                Some(raw) => match raw.parse::<u64>() {
                    Ok(value) => value,
                    Err(_) => {
                        return usage_error(
                            Scope::Session,
                            format!("--since must be a byte offset, not {raw:?}"),
                        );
                    }
                },
            };
            Command::Session(Session::Read {
                host: operands[0].clone(),
                session: operands[1].clone(),
                since,
                timeout_nanos: match timeout(&parsed, READ_TIMEOUT) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Session, message),
                },
            })
        }
        "recover" => {
            if let Err(message) = exact(2, "<host> <session>") {
                return usage_error(Scope::Session, message);
            }
            Command::Session(Session::Recover {
                host: operands[0].clone(),
                session: operands[1].clone(),
                timeout_nanos: match timeout(&parsed, RECOVER_TIMEOUT) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Session, message),
                },
            })
        }
        "close" => {
            if let Err(message) = exact(2, "<host> <session>") {
                return usage_error(Scope::Session, message);
            }
            Command::Session(Session::Close {
                host: operands[0].clone(),
                session: operands[1].clone(),
            })
        }
        // Attaching needs a terminal, so there is no envelope it could honestly
        // carry; v2 reports it as a usage error under its real operation name.
        "attach" => {
            if let Err(message) = exact(2, "<host> <session>") {
                return usage_error(Scope::Session, message);
            }
            Command::Refused {
                operation: "session.attach",
                message: "session attach is interactive and is not implemented by this build"
                    .to_string(),
            }
        }
        other => usage_error(
            Scope::Session,
            format!("unknown command {other} for \"rhost session\""),
        ),
    }
}

/// The flags each leaf accepts, so its `--help` page and its parse read one table.
pub(crate) fn flags(leaf: &str) -> &'static [FlagSpec] {
    match leaf {
        "create" => CREATE_FLAGS,
        "list" => LIST_FLAGS,
        "exec" => EXEC_FLAGS,
        "send" => SEND_FLAGS,
        "read" => READ_FLAGS,
        "recover" => RECOVER_FLAGS,
        // `close` and `attach` take operands only.
        _ => PLAIN_FLAGS,
    }
}

/// `0` means the operation's own default, and a negative budget is a
/// configuration error rather than an instant deadline.
fn timeout(parsed: &Parsed, default: Duration) -> Result<i64, String> {
    match parsed.value("timeout") {
        None => Ok(i64::try_from(default.as_nanos()).unwrap_or(i64::MAX)),
        Some(raw) => match parse_duration(raw) {
            Some(nanos) if nanos >= 0 => Ok(if nanos == 0 {
                i64::try_from(default.as_nanos()).unwrap_or(i64::MAX)
            } else {
                nanos
            }),
            _ => Err(format!("invalid duration for --timeout: {raw}")),
        },
    }
}

pub(super) fn budget(nanos: i64) -> Duration {
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(0))
}
