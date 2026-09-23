//! The `audit` command, and the timer the other commands use to write to it.
//!
//! Auditing is fail-open: a write that does not land is reported on stderr and the
//! operation it describes is unaffected, because a full or read-only disk must not
//! make remote work impossible. `RHOST_AUDIT=0` makes the timer inert, so no call
//! site branches on whether auditing is on.

use super::{dto, trail};
use crate::cli::{FlagSpec, Help, JSON, ParsedCommand, Scope, Sink, parse, warn};
use crate::config;
use std::time::Instant;

/// The flags `audit` takes. It reads a local file, so it has no host operand.
pub static AUDIT_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("limit", false, "show at most N recent entries; 0 shows all"),
    FlagSpec::value("host", false, "only entries for this host"),
];

/// How many entries a caller who said nothing gets. The trail is append-only and
/// unbounded, so the default view is the tail.
const DEFAULT_LIMIT: i64 = 20;

pub fn command(argv: &[String], json: bool) -> ParsedCommand<Audit> {
    let parsed = match parse(argv, AUDIT_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Root, message),
    };
    if parsed.bool_flag("help") {
        return ParsedCommand::help_or_error(Scope::Root, Help::Audit, json);
    }
    if !parsed.operands.is_empty() || parsed.dash {
        return ParsedCommand::usage(
            Scope::Root,
            format!(
                "rhost audit accepts 0 arg(s), received {}",
                parsed.operands.len()
            ),
        );
    }
    let limit = match parsed.value("limit") {
        None => DEFAULT_LIMIT,
        Some(raw) => match raw.parse::<i64>() {
            Ok(value) if value >= 0 => value,
            _ => {
                return ParsedCommand::usage(
                    Scope::Root,
                    format!("--limit must be zero or a positive integer, not {raw:?}"),
                );
            }
        },
    };
    ParsedCommand::Run(Audit {
        limit: limit as usize,
        host: parsed.value("host").unwrap_or("").to_string(),
    })
}

/// Everything `rhost audit` accepted.
pub struct Audit {
    /// At most this many of the most recent entries; `0` means all of them.
    pub limit: usize,
    /// Empty means every host.
    pub host: String,
}

pub fn run(sink: &mut Sink, limit: usize, host: &str, json: bool) -> u8 {
    let path = trail::path(&config::v4_state_dir());
    let entries = match trail::read_selected(&path, host, limit) {
        Ok(entries) => entries,
        Err(error) => {
            return crate::cli::Failure::new(
                "audit",
                "",
                "CONFIG_INVALID",
                format!("cannot read audit log: {error}"),
            )
            .deliver(sink, json);
        }
    };
    if json {
        sink.envelope(&dto::audit(&path, &entries));
        return 0;
    }
    if entries.is_empty() {
        sink.line(&format!("no audit entries ({})", path.display()));
        return 0;
    }
    for entry in &entries {
        let status = if entry.ok {
            "ok".to_string()
        } else if entry.error_code.is_empty() {
            "failed".to_string()
        } else {
            entry.error_code.clone()
        };
        sink.line(&format!(
            "{}  {:<10} {:<18} {:<8} {}",
            entry.time,
            dash(&entry.host),
            entry.operation,
            status,
            entry.command_summary
        ));
    }
    0
}

fn dash(text: &str) -> &str {
    if text.is_empty() { "-" } else { text }
}

/// One operation, timed, and written to the trail when the caller says how it
/// ended. A recorder that cannot write never fails the operation.
pub struct Timer {
    start: Instant,
    recorder: trail::Recorder,
    operation: &'static str,
    host: String,
}

impl Timer {
    pub fn start(operation: &'static str, host: &str) -> Self {
        Self {
            start: Instant::now(),
            recorder: trail::Recorder::from_env(&config::v4_state_dir()),
            operation,
            host: host.to_string(),
        }
    }

    /// Records an operation that completed as designed. `exit_code` is the remote
    /// command's own status when it has one, and `None` for operations whose
    /// answer is not a status.
    pub fn succeeded(&self, cwd: &str, command_summary: &str, exit_code: Option<u8>) {
        self.write(&trail::Entry::completed(
            self.operation,
            &self.host,
            cwd,
            command_summary,
            exit_code,
            self.elapsed_ms(),
        ));
    }

    /// Records a refusal or a failure, keeping the code an agent would branch on.
    pub fn failed(&self, error_code: &str) {
        self.write(&trail::Entry::refused(
            self.operation,
            &self.host,
            error_code,
            self.elapsed_ms(),
        ));
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn write(&self, entry: &trail::Entry) {
        if let Err(error) = self.recorder.record(entry) {
            warn(&format!("rhost: audit: {error}"));
        }
    }
}

/// Records one entry the caller has already timed and described, used where one
/// command covers several operations (a batch) and each one has its own duration.
pub fn record(entry: trail::Entry) {
    let recorder = trail::Recorder::from_env(&config::v4_state_dir());
    if let Err(error) = recorder.record(&entry) {
        warn(&format!("rhost: audit: {error}"));
    }
}
