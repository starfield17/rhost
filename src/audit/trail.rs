//! The local audit trail: one JSON line per remote operation.
//!
//! It records bounded operation metadata — never an environment map, never file
//! contents — so a caller can answer "what did this machine do to that host" after
//! the fact. Logging is deliberately fail-open: a full or read-only disk must not
//! make remote work impossible, so a write failure is reported and the operation
//! carries on. `RHOST_AUDIT=0` (or false/no/off) turns it off entirely.

use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Turns auditing on or off. Anything but 0/false/no/off leaves it on.
pub const ENV_VAR: &str = "RHOST_AUDIT";

/// The log's file name under the state root.
pub const FILE_NAME: &str = "audit.jsonl";

/// A command summary answers "what ran", not "reproduce this argv", so it is one
/// bounded line.
const MAX_COMMAND: usize = 200;

/// One audited operation. The JSON field names are a stable surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub time: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u8>,
    pub duration_ms: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error_code: String,
}

impl Entry {
    /// An operation that completed as designed. A remote command may still have
    /// exited non-zero: what `ok` reports is the adapter's own outcome.
    pub fn completed(
        operation: &str,
        host: &str,
        cwd: &str,
        command_summary: &str,
        exit_code: Option<u8>,
        duration_ms: u64,
    ) -> Self {
        Self {
            time: crate::clock::now_rfc3339(),
            host: host.to_string(),
            operation: operation.to_string(),
            cwd: cwd.to_string(),
            command_summary: summarize(command_summary),
            exit_code,
            duration_ms,
            ok: true,
            error_code: String::new(),
        }
    }

    /// An operation rhost refused, or could not carry out.
    pub fn refused(operation: &str, host: &str, error_code: &str, duration_ms: u64) -> Self {
        Self {
            time: crate::clock::now_rfc3339(),
            host: host.to_string(),
            operation: operation.to_string(),
            cwd: String::new(),
            command_summary: String::new(),
            exit_code: None,
            duration_ms,
            ok: false,
            error_code: error_code.to_string(),
        }
    }
}

/// Whether auditing is on. Unset means on.
pub fn enabled() -> bool {
    match std::env::var(ENV_VAR) {
        Ok(value) => !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        ),
        Err(_) => true,
    }
}

/// The trail's path inside the state root this version owns. v4 keeps its own
/// file (CONTRACT.md PERSIST-002): the version never reads, appends to or deletes
/// the trail a v3 installation wrote.
pub fn path(namespace_dir: &Path) -> PathBuf {
    namespace_dir.join(FILE_NAME)
}

/// Appends entries to one log. A disabled recorder discards them, so no call site
/// has to branch on whether auditing is on.
#[derive(Debug, Clone)]
pub struct Recorder {
    path: PathBuf,
    enabled: bool,
}

impl Recorder {
    pub fn from_env(namespace_dir: &Path) -> Self {
        Self {
            path: path(namespace_dir),
            enabled: enabled(),
        }
    }

    /// Appends one entry as a single line.
    ///
    /// The file is opened with `O_APPEND` so concurrent rhost processes interleave
    /// whole lines rather than corrupting each other's, and it is created 0600 in
    /// a 0700 directory. A path that is a symlink or not a regular file is refused
    /// rather than followed: the trail is rhost's own record.
    pub fn record(&self, entry: &Entry) -> io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        if let Ok(info) = std::fs::symlink_metadata(&self.path) {
            if info.file_type().is_symlink() || !info.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is not a regular file", self.path.display()),
                ));
            }
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
        }
        let mut line = serde_json::to_vec(entry).map_err(io::Error::other)?;
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&self.path)?;
        file.write_all(&line)
    }
}

/// Collapses a command to one bounded line: an entry is one JSON line, so
/// newlines become spaces, and a long command is cut at a rune boundary.
pub fn summarize(command: &str) -> String {
    let collapsed = command.replace('\n', " ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= MAX_COMMAND {
        return trimmed.to_string();
    }
    let mut cut: String = trimmed.chars().take(MAX_COMMAND).collect();
    cut.push('…');
    cut
}

/// Reads the trail. A missing file is an empty log, not a failure; a line that
/// does not parse is skipped rather than making the whole trail unreadable, which
/// matters because other processes append to it while it is being read.
pub fn read(path: &Path) -> io::Result<Vec<Entry>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
        .collect())
}

pub fn filter_host(entries: Vec<Entry>, host: &str) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|entry| entry.host == host)
        .collect()
}

/// The last `limit` entries, or all of them when `limit` is zero.
pub fn keep_last(entries: Vec<Entry>, limit: usize) -> Vec<Entry> {
    if limit == 0 || entries.len() <= limit {
        return entries;
    }
    let start = entries.len() - limit;
    entries[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rhost-audit-unit-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn entries_append_as_one_json_line_each_with_a_private_mode() {
        let dir = scratch();
        let recorder = Recorder {
            path: path(&dir),
            enabled: true,
        };
        recorder
            .record(&Entry::completed(
                "exec",
                "gpu",
                "/tmp",
                "pytest -q",
                Some(3),
                831,
            ))
            .unwrap_or_else(|error| panic!("{error}"));
        recorder
            .record(&Entry::refused("doctor", "gpu", "SSH_AUTH_FAILED", 12))
            .unwrap_or_else(|error| panic!("{error}"));
        let text = std::fs::read_to_string(path(&dir)).unwrap_or_default();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains(r#""command_summary":"pytest -q""#), "{text}");
        assert!(text.contains(r#""error_code":"SSH_AUTH_FAILED""#), "{text}");
        let mode = std::fs::metadata(path(&dir))
            .map(|info| info.permissions().mode() & 0o777)
            .unwrap_or(0);
        assert_eq!(mode, 0o600);
        let entries = read(&path(&dir)).unwrap_or_default();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].exit_code, Some(3));
        assert!(entries[0].ok && !entries[1].ok);
        assert_eq!(entries[0].time.len(), 20, "{}", entries[0].time);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_disabled_recorder_writes_nothing_and_a_missing_log_is_empty() {
        let dir = scratch();
        let recorder = Recorder {
            path: path(&dir),
            enabled: false,
        };
        recorder
            .record(&Entry::completed("exec", "gpu", "", "", None, 1))
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(!path(&dir).exists());
        assert!(read(&path(&dir)).unwrap_or_default().is_empty());
        assert!(!dir.exists(), "a disabled recorder creates nothing");
    }

    #[test]
    fn a_corrupt_line_is_skipped_and_filters_keep_the_tail() {
        let dir = scratch();
        std::fs::create_dir_all(&dir).unwrap_or_else(|error| panic!("{error}"));
        let good = serde_json::to_string(&Entry::completed("exec", "gpu", "", "true", Some(0), 1))
            .unwrap_or_default();
        std::fs::write(path(&dir), format!("{good}\nnot json\n\n{good}\n"))
            .unwrap_or_else(|error| panic!("{error}"));
        let entries = read(&path(&dir)).unwrap_or_default();
        assert_eq!(entries.len(), 2);
        assert_eq!(filter_host(entries.clone(), "other").len(), 0);
        assert_eq!(filter_host(entries.clone(), "gpu").len(), 2);
        assert_eq!(keep_last(entries.clone(), 1).len(), 1);
        assert_eq!(keep_last(entries.clone(), 0).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_command_summary_is_one_bounded_line() {
        assert_eq!(summarize("  echo hi\n  "), "echo hi");
        assert_eq!(summarize("a\nb"), "a b");
        let long = "x".repeat(MAX_COMMAND + 10);
        let summary = summarize(&long);
        assert_eq!(summary.chars().count(), MAX_COMMAND + 1);
        assert!(summary.ends_with('…'));
        // A multi-byte rune is never cut in half.
        let wide = "é".repeat(MAX_COMMAND + 5);
        assert!(summarize(&wide).ends_with('…'));
    }
}
