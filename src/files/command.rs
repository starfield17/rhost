//! `fs` grammar: the flag tables and the mapping from argv to one [`Fs`].

use super::Fs;
use crate::cli::{FlagSpec, Help, JSON, PLAIN_FLAGS, ParsedCommand, Scope, parse, parse_duration};

pub(crate) static FS_PUT_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "deadline for the operation; 0 uses the 5-minute default",
    ),
    FlagSpec::long("resume", "resume a partial transfer"),
    FlagSpec::long("checksum", "verify with a checksum"),
    FlagSpec::long("parents", "create missing remote directories"),
];
static FS_GET_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "deadline for the operation; 0 uses the 5-minute default",
    ),
    FlagSpec::long("resume", "resume a partial transfer"),
    FlagSpec::long("checksum", "verify with a checksum"),
];
static FS_SYNC_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "deadline for the operation; 0 uses the 5-minute default",
    ),
    FlagSpec::long("delete", "delete remote files missing locally"),
    FlagSpec::long("dry-run", "report changes without applying them"),
    FlagSpec::long("checksum", "verify with a checksum"),
    FlagSpec::value("exclude", false, "remote path pattern to skip (repeatable)"),
];
static FS_BATCH_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "per-item deadline; 0 uses the 5-minute default",
    ),
    FlagSpec::value("manifest", false, "local JSON manifest of copies"),
];
static FS_READ_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "deadline for the operation; 0 uses the 60-second default",
    ),
    FlagSpec::value(
        "max-bytes",
        false,
        "maximum bytes to return; default 256 KiB, max 8 MiB",
    ),
    FlagSpec::value("start", false, "first line to return (1-based)"),
    FlagSpec::value("lines", false, "number of lines to return"),
];
static FS_WRITE_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "deadline for the operation; 0 uses the 60-second default",
    ),
    FlagSpec::value("from", false, "local file to upload, or - for stdin"),
    FlagSpec::value(
        "if-hash",
        false,
        "refuse unless the current SHA-256 matches",
    ),
    FlagSpec::value(
        "mode",
        false,
        "3- or 4-digit octal mode; new files default 0600, replacements keep theirs",
    ),
    FlagSpec::long("parents", "create missing remote directories"),
];
static FS_PATCH_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value(
        "timeout",
        false,
        "deadline for the operation; 0 uses the 60-second default",
    ),
    FlagSpec::value("patch", false, "local patch file, or - for stdin"),
    FlagSpec::value(
        "if-hash",
        false,
        "refuse unless the current SHA-256 matches",
    ),
];

/// Every flag the `fs` group knows, used only to find the leaf before its own
/// table parses the same argv again. A flag that belongs to another leaf is
/// therefore still rejected — by the second parse, which is the one that counts.
/// The help text is unused here; the leaf's own table carries it.
static FS_ANY_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false, ""),
    FlagSpec::value("max-bytes", false, ""),
    FlagSpec::value("start", false, ""),
    FlagSpec::value("lines", false, ""),
    FlagSpec::value("from", false, ""),
    FlagSpec::value("if-hash", false, ""),
    FlagSpec::value("mode", false, ""),
    FlagSpec::value("patch", false, ""),
    FlagSpec::value("manifest", false, ""),
    FlagSpec::value("exclude", false, ""),
    FlagSpec::long("resume", ""),
    FlagSpec::long("checksum", ""),
    FlagSpec::long("parents", ""),
    FlagSpec::long("delete", ""),
    FlagSpec::long("dry-run", ""),
];

/// Defaults for `fs` timeouts: a copy's duration belongs to the size of the
/// data, while an editing helper performs one bounded action.
const TRANSFER_TIMEOUT_NANOS: i64 = 5 * 60 * 1_000_000_000;
const HELPER_TIMEOUT_NANOS: i64 = 60 * 1_000_000_000;

/// `fs` and its leaves. The leaf is located first with the group's whole flag
/// set, then the same argv is parsed again with the leaf's own table, so a flag
/// from a sibling operation is refused rather than silently ignored.
pub fn command(argv: &[String], json: bool) -> ParsedCommand<Fs> {
    let scanned = match parse(argv, FS_ANY_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Fs, message),
    };
    let leaf = scanned.operand(0).map(str::to_string);
    if scanned.bool_flag("help") {
        if json {
            return ParsedCommand::help_or_error(Scope::Fs, Help::Group(Scope::Fs), json);
        }
        return match &leaf {
            Some(leaf) => ParsedCommand::HelpLeaf(Scope::Fs, leaf.clone()),
            None => ParsedCommand::Help(Help::Group(Scope::Fs)),
        };
    }
    let Some(leaf) = leaf else {
        return ParsedCommand::usage(Scope::Fs, "rhost fs needs a subcommand".to_string());
    };
    let parsed = match parse(argv, leaf_flags(&leaf)) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Fs, message),
    };
    let flag_or = |name: &str| parsed.value(name).map(str::to_string);
    let timeout = |default: i64| match parsed.value("timeout") {
        Some(raw) => match parse_duration(raw) {
            Some(nanos) if nanos >= 0 => Ok(if nanos == 0 { default } else { nanos }),
            _ => Err(format!("invalid duration for --timeout: {raw}")),
        },
        None => Ok(default),
    };
    // Every leaf but `batch` takes operands after the leaf itself.
    let operands = &parsed.operands[1.min(parsed.operands.len())..];
    let exact = |count: usize, names: &str| -> Result<(), String> {
        if operands.len() == count {
            return Ok(());
        }
        Err(format!(
            "rhost fs {leaf} accepts {count} arg(s) {names}, received {}",
            operands.len()
        ))
    };
    match leaf.as_str() {
        "put" => {
            if let Err(message) = exact(3, "<host> <local> <remote>") {
                return ParsedCommand::usage(Scope::Root, message);
            }
            ParsedCommand::Run(Fs::Put {
                host: operands[0].clone(),
                local: operands[1].clone(),
                remote: operands[2].clone(),
                timeout_nanos: match timeout(TRANSFER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return ParsedCommand::usage(Scope::Root, message),
                },
                resume: parsed.bool_flag("resume"),
                checksum: parsed.bool_flag("checksum"),
                parents: parsed.bool_flag("parents"),
            })
        }
        "get" => {
            if let Err(message) = exact(3, "<host> <remote> <local>") {
                return ParsedCommand::usage(Scope::Root, message);
            }
            ParsedCommand::Run(Fs::Get {
                host: operands[0].clone(),
                remote: operands[1].clone(),
                local: operands[2].clone(),
                timeout_nanos: match timeout(TRANSFER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return ParsedCommand::usage(Scope::Root, message),
                },
                resume: parsed.bool_flag("resume"),
                checksum: parsed.bool_flag("checksum"),
            })
        }
        "sync" | "mirror" => {
            let mirror = leaf == "mirror";
            let names = if mirror {
                "<host> <remote-dir> <local-dir>"
            } else {
                "<host> <local-dir> <remote-dir>"
            };
            if let Err(message) = exact(3, names) {
                return ParsedCommand::usage(Scope::Root, message);
            }
            ParsedCommand::Run(Fs::Sync {
                mirror,
                host: operands[0].clone(),
                source: operands[1].clone(),
                destination: operands[2].clone(),
                timeout_nanos: match timeout(TRANSFER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return ParsedCommand::usage(Scope::Root, message),
                },
                delete: parsed.bool_flag("delete"),
                dry_run: parsed.bool_flag("dry-run"),
                checksum: parsed.bool_flag("checksum"),
                excludes: parsed.all("exclude"),
            })
        }
        "batch" => {
            if let Err(message) = exact(1, "<host>") {
                return ParsedCommand::usage(Scope::Root, message);
            }
            let Some(manifest) = flag_or("manifest") else {
                return ParsedCommand::usage(Scope::Root, "fs batch needs --manifest".to_string());
            };
            ParsedCommand::Run(Fs::Batch {
                host: operands[0].clone(),
                manifest,
                timeout_nanos: match timeout(TRANSFER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return ParsedCommand::usage(Scope::Root, message),
                },
            })
        }
        "read" => {
            if let Err(message) = exact(2, "<host> <path>") {
                return ParsedCommand::usage(Scope::Root, message);
            }
            let number = |name: &str, default: u64| -> Result<u64, String> {
                match parsed.value(name) {
                    None => Ok(default),
                    Some(raw) => raw
                        .parse::<u64>()
                        .ok()
                        .filter(|value| *value >= 1)
                        .ok_or_else(|| format!("--{name} must be a positive integer")),
                }
            };
            let max_bytes = match parsed.value("max-bytes") {
                None => 256 * 1024,
                Some(raw) => match raw.parse::<usize>() {
                    Ok(value) if (1..=super::ops::MAX_HELPER_BYTES).contains(&value) => value,
                    _ => {
                        return ParsedCommand::usage(
                            Scope::Root,
                            format!(
                                "--max-bytes must be between 1 and {}",
                                super::ops::MAX_HELPER_BYTES
                            ),
                        );
                    }
                },
            };
            let start = match number("start", 1) {
                Ok(value) => value,
                Err(message) => return ParsedCommand::usage(Scope::Root, message),
            };
            let lines = match number("lines", 200) {
                Ok(value) => value,
                Err(message) => return ParsedCommand::usage(Scope::Root, message),
            };
            ParsedCommand::Run(Fs::Read {
                host: operands[0].clone(),
                path: operands[1].clone(),
                start,
                lines,
                max_bytes,
                timeout_nanos: match timeout(HELPER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return ParsedCommand::usage(Scope::Root, message),
                },
            })
        }
        "write" => {
            if let Err(message) = exact(2, "<host> <path>") {
                return ParsedCommand::usage(Scope::Root, message);
            }
            ParsedCommand::Run(Fs::Write {
                host: operands[0].clone(),
                path: operands[1].clone(),
                from: flag_or("from").unwrap_or_else(|| "-".to_string()),
                if_hash: flag_or("if-hash"),
                parents: parsed.bool_flag("parents"),
                mode: flag_or("mode"),
                timeout_nanos: match timeout(HELPER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return ParsedCommand::usage(Scope::Root, message),
                },
            })
        }
        "patch" => {
            if let Err(message) = exact(2, "<host> <path>") {
                return ParsedCommand::usage(Scope::Root, message);
            }
            ParsedCommand::Run(Fs::Patch {
                host: operands[0].clone(),
                path: operands[1].clone(),
                patch: flag_or("patch").unwrap_or_else(|| "-".to_string()),
                if_hash: flag_or("if-hash"),
                timeout_nanos: match timeout(HELPER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return ParsedCommand::usage(Scope::Root, message),
                },
            })
        }
        other => ParsedCommand::usage(
            Scope::Root,
            format!("unknown command {other} for \"rhost fs\""),
        ),
    }
}

/// The flags each leaf accepts, so its `--help` page and its parse read one table.
pub fn leaf_flags(leaf: &str) -> &'static [FlagSpec] {
    match leaf {
        "put" => FS_PUT_FLAGS,
        "get" => FS_GET_FLAGS,
        "sync" | "mirror" => FS_SYNC_FLAGS,
        "batch" => FS_BATCH_FLAGS,
        "read" => FS_READ_FLAGS,
        "write" => FS_WRITE_FLAGS,
        "patch" => FS_PATCH_FLAGS,
        // An unknown leaf has no flags to reject yet; the match below reports it
        // as an unknown command rather than as a flag problem.
        _ => PLAIN_FLAGS,
    }
}
