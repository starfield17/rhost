//! `fs` grammar: the flag tables and the mapping from argv to one [`Fs`].

use crate::app::files as app_files;
use crate::cli::grammar::{
    FlagSpec, JSON, PLAIN_FLAGS, help_or_error, parse, parse_duration, usage_error,
};
use crate::cli::{Command, Scope};

/// Every file operation the parser can hand to [`run`].
pub enum Fs {
    Put {
        host: String,
        local: String,
        remote: String,
        timeout_nanos: i64,
        resume: bool,
        checksum: bool,
        parents: bool,
    },
    Get {
        host: String,
        remote: String,
        local: String,
        timeout_nanos: i64,
        resume: bool,
        checksum: bool,
    },
    /// `sync` pushes and `mirror` pulls; the direction is part of the operation
    /// name, never a flag a caller could get backwards.
    Sync {
        mirror: bool,
        host: String,
        source: String,
        destination: String,
        timeout_nanos: i64,
        delete: bool,
        dry_run: bool,
        checksum: bool,
        excludes: Vec<String>,
    },
    Batch {
        host: String,
        manifest: String,
        timeout_nanos: i64,
    },
    Read {
        host: String,
        path: String,
        start: u64,
        lines: u64,
        max_bytes: usize,
        timeout_nanos: i64,
    },
    Write {
        host: String,
        path: String,
        /// `-` means stdin, read before anything is sent.
        from: String,
        if_hash: Option<String>,
        parents: bool,
        mode: Option<String>,
        timeout_nanos: i64,
    },
    Patch {
        host: String,
        path: String,
        patch: String,
        if_hash: Option<String>,
        timeout_nanos: i64,
    },
}

pub(crate) static FS_PUT_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false),
    FlagSpec::long("resume"),
    FlagSpec::long("checksum"),
    FlagSpec::long("parents"),
];
static FS_GET_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false),
    FlagSpec::long("resume"),
    FlagSpec::long("checksum"),
];
static FS_SYNC_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false),
    FlagSpec::long("delete"),
    FlagSpec::long("dry-run"),
    FlagSpec::long("checksum"),
    FlagSpec::value("exclude", false),
];
static FS_BATCH_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false),
    FlagSpec::value("manifest", false),
];
static FS_READ_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false),
    FlagSpec::value("max-bytes", false),
    FlagSpec::value("start", false),
    FlagSpec::value("lines", false),
];
static FS_WRITE_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false),
    FlagSpec::value("from", false),
    FlagSpec::value("if-hash", false),
    FlagSpec::value("mode", false),
    FlagSpec::long("parents"),
];
static FS_PATCH_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false),
    FlagSpec::value("patch", false),
    FlagSpec::value("if-hash", false),
];

/// Every flag the `fs` group knows, used only to find the leaf before its own
/// table parses the same argv again. A flag that belongs to another leaf is
/// therefore still rejected — by the second parse, which is the one that counts.
static FS_ANY_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false),
    FlagSpec::value("max-bytes", false),
    FlagSpec::value("start", false),
    FlagSpec::value("lines", false),
    FlagSpec::value("from", false),
    FlagSpec::value("if-hash", false),
    FlagSpec::value("mode", false),
    FlagSpec::value("patch", false),
    FlagSpec::value("manifest", false),
    FlagSpec::value("exclude", false),
    FlagSpec::long("resume"),
    FlagSpec::long("checksum"),
    FlagSpec::long("parents"),
    FlagSpec::long("delete"),
    FlagSpec::long("dry-run"),
];

/// Defaults for `fs` timeouts: a copy's duration belongs to the size of the
/// data, while an editing helper performs one bounded action.
const TRANSFER_TIMEOUT_NANOS: i64 = 5 * 60 * 1_000_000_000;
const HELPER_TIMEOUT_NANOS: i64 = 60 * 1_000_000_000;

/// `fs` and its leaves. The leaf is located first with the group's whole flag
/// set, then the same argv is parsed again with the leaf's own table, so a flag
/// from a sibling operation is refused rather than silently ignored.
pub(crate) fn command(argv: &[String], json: bool) -> Command {
    let scanned = match parse(argv, FS_ANY_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Fs, message),
    };
    if scanned.bool_flag("help") {
        return help_or_error(Scope::Fs, json);
    }
    let Some(leaf) = scanned.operand(0).map(str::to_string) else {
        return usage_error(Scope::Fs, "rhost fs needs a subcommand".to_string());
    };
    let parsed = match parse(argv, fs_flags(&leaf)) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(Scope::Fs, message),
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
                return usage_error(Scope::Root, message);
            }
            Command::Fs(Fs::Put {
                host: operands[0].clone(),
                local: operands[1].clone(),
                remote: operands[2].clone(),
                timeout_nanos: match timeout(TRANSFER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Root, message),
                },
                resume: parsed.bool_flag("resume"),
                checksum: parsed.bool_flag("checksum"),
                parents: parsed.bool_flag("parents"),
            })
        }
        "get" => {
            if let Err(message) = exact(3, "<host> <remote> <local>") {
                return usage_error(Scope::Root, message);
            }
            Command::Fs(Fs::Get {
                host: operands[0].clone(),
                remote: operands[1].clone(),
                local: operands[2].clone(),
                timeout_nanos: match timeout(TRANSFER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Root, message),
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
                return usage_error(Scope::Root, message);
            }
            Command::Fs(Fs::Sync {
                mirror,
                host: operands[0].clone(),
                source: operands[1].clone(),
                destination: operands[2].clone(),
                timeout_nanos: match timeout(TRANSFER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Root, message),
                },
                delete: parsed.bool_flag("delete"),
                dry_run: parsed.bool_flag("dry-run"),
                checksum: parsed.bool_flag("checksum"),
                excludes: parsed.all("exclude"),
            })
        }
        "batch" => {
            if let Err(message) = exact(1, "<host>") {
                return usage_error(Scope::Root, message);
            }
            let Some(manifest) = flag_or("manifest") else {
                return usage_error(Scope::Root, "fs batch needs --manifest".to_string());
            };
            Command::Fs(Fs::Batch {
                host: operands[0].clone(),
                manifest,
                timeout_nanos: match timeout(TRANSFER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Root, message),
                },
            })
        }
        "read" => {
            if let Err(message) = exact(2, "<host> <path>") {
                return usage_error(Scope::Root, message);
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
                    Ok(value) if (1..=app_files::MAX_HELPER_BYTES).contains(&value) => value,
                    _ => {
                        return usage_error(
                            Scope::Root,
                            format!(
                                "--max-bytes must be between 1 and {}",
                                app_files::MAX_HELPER_BYTES
                            ),
                        );
                    }
                },
            };
            let start = match number("start", 1) {
                Ok(value) => value,
                Err(message) => return usage_error(Scope::Root, message),
            };
            let lines = match number("lines", 200) {
                Ok(value) => value,
                Err(message) => return usage_error(Scope::Root, message),
            };
            Command::Fs(Fs::Read {
                host: operands[0].clone(),
                path: operands[1].clone(),
                start,
                lines,
                max_bytes,
                timeout_nanos: match timeout(HELPER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Root, message),
                },
            })
        }
        "write" => {
            if let Err(message) = exact(2, "<host> <path>") {
                return usage_error(Scope::Root, message);
            }
            Command::Fs(Fs::Write {
                host: operands[0].clone(),
                path: operands[1].clone(),
                from: flag_or("from").unwrap_or_else(|| "-".to_string()),
                if_hash: flag_or("if-hash"),
                parents: parsed.bool_flag("parents"),
                mode: flag_or("mode"),
                timeout_nanos: match timeout(HELPER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Root, message),
                },
            })
        }
        "patch" => {
            if let Err(message) = exact(2, "<host> <path>") {
                return usage_error(Scope::Root, message);
            }
            Command::Fs(Fs::Patch {
                host: operands[0].clone(),
                path: operands[1].clone(),
                patch: flag_or("patch").unwrap_or_else(|| "-".to_string()),
                if_hash: flag_or("if-hash"),
                timeout_nanos: match timeout(HELPER_TIMEOUT_NANOS) {
                    Ok(nanos) => nanos,
                    Err(message) => return usage_error(Scope::Root, message),
                },
            })
        }
        other => usage_error(
            Scope::Root,
            format!("unknown command {other} for \"rhost fs\""),
        ),
    }
}

fn fs_flags(leaf: &str) -> &'static [FlagSpec] {
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
