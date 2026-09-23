//! Parsers for the session helper's line protocol.
//!
//! The helper is rhost's own program, so this is not a foreign dialect to be
//! tolerated: every field is either exactly what the script promised, or the
//! whole answer is a protocol failure. A missing, duplicated, malformed or
//! contradictory field therefore becomes `SESSION_UNHEALTHY` — never an empty
//! successful result (SESSION-006).

use super::super::Meta;
use crate::base64;

/// One row of `session list`: the record, and whether its tmux session is there.
#[derive(Debug, Clone)]
pub struct ListEntry {
    pub id: String,
    pub alive: bool,
    pub meta: Meta,
    /// The absolute directory the session was actually created in, from the
    /// sidecar this build writes. `None` for a record that predates it, so the
    /// caller falls back to the metadata's literal `initial_cwd`.
    pub resolved_cwd: Option<String>,
}

/// Every failure a helper reports, as a closed set.
///
/// A code outside this list is not "another kind of failure" to be guessed at:
/// it is the helper being wrong, and it maps to the same protocol answer as an
/// unparseable line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperFailure {
    NoSession,
    SessionDied,
    Timeout,
    Locked,
    InputFailed,
    InputUncertain,
    CloseFailed,
    Busy,
    UnknownForeground,
    NoTty,
    NotReady,
    NoTmux,
    NoFlock,
    NameInUse,
    InvalidCwd,
    NewFailed,
    Protocol,
    Unknown(String),
}

impl HelperFailure {
    pub fn from_code(code: &str) -> Self {
        match code {
            "nosession" => Self::NoSession,
            "sessiondied" => Self::SessionDied,
            "timeout" => Self::Timeout,
            "locked" => Self::Locked,
            "inputfailed" => Self::InputFailed,
            "inputuncertain" => Self::InputUncertain,
            "closefailed" => Self::CloseFailed,
            "busy" => Self::Busy,
            "unknownfg" => Self::UnknownForeground,
            "notty" => Self::NoTty,
            "notready" => Self::NotReady,
            "notmux" => Self::NoTmux,
            "noflock" => Self::NoFlock,
            "nameinuse" => Self::NameInUse,
            "invalidcwd" => Self::InvalidCwd,
            "newfailed" => Self::NewFailed,
            "protocol" => Self::Protocol,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn code(&self) -> &str {
        match self {
            Self::NoSession => "nosession",
            Self::SessionDied => "sessiondied",
            Self::Timeout => "timeout",
            Self::Locked => "locked",
            Self::InputFailed => "inputfailed",
            Self::InputUncertain => "inputuncertain",
            Self::CloseFailed => "closefailed",
            Self::Busy => "busy",
            Self::UnknownForeground => "unknownfg",
            Self::NoTty => "notty",
            Self::NotReady => "notready",
            Self::NoTmux => "notmux",
            Self::NoFlock => "noflock",
            Self::NameInUse => "nameinuse",
            Self::InvalidCwd => "invalidcwd",
            Self::NewFailed => "newfailed",
            Self::Protocol => "protocol",
            Self::Unknown(code) => code,
        }
    }
}

/// One exec attempt: either token-bound completion, or a refusal that says what
/// happened to the session.
#[derive(Debug, Clone)]
pub enum ExecResult {
    Completed {
        id: String,
        /// The raw terminal bytes between the start of this command and its
        /// marker, before any presentation-layer stripping.
        output: Vec<u8>,
        exit_code: u8,
    },
    Failed {
        failure: HelperFailure,
        /// Present only when the helper resolved a session before failing.
        id: Option<String>,
        /// The pane's foreground command, when the refusal named it.
        foreground: Option<String>,
        /// Set only by a timeout, and only when the helper observed the pane
        /// returning to a prompt.
        recovered: bool,
    },
}

/// One incremental read: a cursor window onto the pane log.
#[derive(Debug, Clone)]
pub struct ReadResult {
    pub id: String,
    pub from: u64,
    pub next: u64,
    pub size: u64,
    pub data: Vec<u8>,
}

/// The first value of one `KEY=value` line, or `None` when the helper printed no
/// line for that key.
pub fn field<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(key).map(str::trim))
}

fn failure_of(stdout: &str) -> Option<HelperFailure> {
    field(stdout, "RHOST_ERR=").map(HelperFailure::from_code)
}

/// Parses `session list` output. Lines that are not records are ignored: a stray
/// line is not a session, and a record this version cannot read is not one it may
/// invent.
pub fn parse_list(stdout: &str) -> Result<Vec<ListEntry>, HelperFailure> {
    if let Some(failure) = failure_of(stdout) {
        return Err(failure);
    }
    let mut entries: Vec<ListEntry> = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("RHOST_META\t") else {
            continue;
        };
        // The fourth field is the resolved cwd sidecar. A record written by an
        // earlier build has only three, and that is not an error: the metadata's
        // literal value is the fallback, and no old record is rewritten.
        let mut parts = rest.splitn(4, '\t');
        let (Some(id), Some(alive), Some(meta)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let resolved_cwd = parts
            .next()
            .and_then(base64::decode)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|value| !value.is_empty());
        let Some(decoded) = base64::decode(meta) else {
            continue;
        };
        let Ok(meta) = serde_json::from_slice::<Meta>(&decoded) else {
            continue;
        };
        entries.push(ListEntry {
            id: id.to_string(),
            alive: alive == "yes",
            meta,
            resolved_cwd,
        });
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(entries)
}

/// Parses `session exec` output against the token this invocation submitted.
///
/// The token is what makes completion evidence *this* invocation's: output
/// without it, output carrying another token, or two answers in one stream are
/// all protocol failures rather than a status to report.
pub fn parse_exec(stdout: &str, expected_token: &str) -> ExecResult {
    if let Some(failure) = failure_of(stdout) {
        return ExecResult::Failed {
            failure,
            id: field(stdout, "RHOST_ID=")
                .filter(|id| !id.is_empty())
                .map(str::to_string),
            foreground: field(stdout, "RHOST_FG=")
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            recovered: field(stdout, "RHOST_RECOVERED=") == Some("1"),
        };
    }
    let mut id: Option<String> = None;
    let mut tokens: Vec<&str> = Vec::new();
    let mut exits: Vec<&str> = Vec::new();
    let mut outputs: Vec<&str> = Vec::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("RHOST_TOKEN=") {
            tokens.push(value);
        } else if let Some(value) = line.strip_prefix("RHOST_EXIT=") {
            exits.push(value);
        } else if let Some(value) = line.strip_prefix("RHOST_OUTPUT=") {
            outputs.push(value);
        } else if let Some(value) = line.strip_prefix("RHOST_ID=") {
            // A second identity is a contradiction, not the last word.
            if id.is_some() || value.is_empty() {
                return protocol_failure(id);
            }
            id = Some(value.to_string());
        } else if line.is_empty() {
        } else {
            return protocol_failure(id);
        }
    }
    let Some(id) = id else {
        return protocol_failure(None);
    };
    let ([token], [exit], [output]) = (tokens.as_slice(), exits.as_slice(), outputs.as_slice())
    else {
        return protocol_failure(Some(id));
    };
    if *token != expected_token {
        return protocol_failure(Some(id));
    }
    let Some(exit_code) = canonical_exit(exit) else {
        return protocol_failure(Some(id));
    };
    let Some(output) = base64::decode(output.trim()) else {
        return protocol_failure(Some(id));
    };
    ExecResult::Completed {
        id,
        output,
        exit_code,
    }
}

fn protocol_failure(id: Option<String>) -> ExecResult {
    ExecResult::Failed {
        failure: HelperFailure::Protocol,
        id,
        foreground: None,
        recovered: false,
    }
}

/// A status is only a status when it is written as one: `07` describes a
/// different byte stream than `7` does, and a negative or oversized value is not
/// an exit code at all.
fn canonical_exit(text: &str) -> Option<u8> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let code: u16 = text.parse().ok()?;
    if code > 255 || code.to_string() != text {
        return None;
    }
    Some(code as u8)
}

/// Parses `session read` output. Each cursor field must appear exactly once and
/// the three must agree; the content is whatever non-field lines carry.
pub fn parse_read(stdout: &str) -> Result<ReadResult, HelperFailure> {
    if let Some(failure) = failure_of(stdout) {
        return Err(failure);
    }
    let mut id: Option<&str> = None;
    let (mut from, mut next, mut size): (Option<u64>, Option<u64>, Option<u64>) =
        (None, None, None);
    let mut encoded = String::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("RHOST_ID=") {
            if id.is_some() || value.is_empty() {
                return Err(HelperFailure::Protocol);
            }
            id = Some(value);
        } else if let Some(value) = line.strip_prefix("RHOST_FROM=") {
            from = parse_cursor(value, from.is_some())?;
        } else if let Some(value) = line.strip_prefix("RHOST_NEXT=") {
            next = parse_cursor(value, next.is_some())?;
        } else if let Some(value) = line.strip_prefix("RHOST_SIZE=") {
            size = parse_cursor(value, size.is_some())?;
        } else if line.starts_with("RHOST_") {
            return Err(HelperFailure::Protocol);
        } else {
            encoded.push_str(line.trim());
        }
    }
    let (Some(id), Some(from), Some(next), Some(size)) = (id, from, next, size) else {
        return Err(HelperFailure::Protocol);
    };
    if next < from || size < next {
        return Err(HelperFailure::Protocol);
    }
    let data = base64::decode(&encoded).ok_or(HelperFailure::Protocol)?;
    if next - from != data.len() as u64 {
        return Err(HelperFailure::Protocol);
    }
    Ok(ReadResult {
        id: id.to_string(),
        from,
        next,
        size,
        data,
    })
}

fn parse_cursor(text: &str, seen: bool) -> Result<Option<u64>, HelperFailure> {
    if seen || text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(HelperFailure::Protocol);
    }
    text.parse().map(Some).map_err(|_| HelperFailure::Protocol)
}

#[cfg(test)]
#[path = "protocol/tests.rs"]
mod tests;
