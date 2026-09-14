//! Parsers for the session helper's line protocol.
//!
//! The helper is rhost's own program, so this is not a foreign dialect to be
//! tolerated: every field is either exactly what the script promised, or the
//! whole answer is a protocol failure. A missing, duplicated, malformed or
//! contradictory field therefore becomes `SESSION_UNHEALTHY` — never an empty
//! successful result (SESSION-006).

use super::Meta;
use crate::base64;

/// One row of `session list`: the record, and whether its tmux session is there.
#[derive(Debug, Clone)]
pub struct ListEntry {
    pub id: String,
    pub alive: bool,
    pub meta: Meta,
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
        let mut parts = rest.splitn(3, '\t');
        let (Some(id), Some(alive), Some(meta)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
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
mod tests {
    use super::*;

    fn meta_line(id: &str, alive: &str, name: &str) -> String {
        let meta = format!(
            r#"{{"schema_version":1,"id":"{id}","name":"{name}","tmux_session":"rhost_s_{id}","created_at":"now","shell":"bash"}}"#
        );
        format!(
            "RHOST_META\t{id}\t{alive}\t{}\n",
            base64::encode(meta.as_bytes())
        )
    }

    #[test]
    fn list_keeps_records_sorts_them_and_reports_a_helper_refusal() {
        let stdout = format!(
            "noise\n{}{}{}",
            meta_line("s_b", "no", "beta"),
            meta_line("s_a", "yes", "alpha"),
            "RHOST_META\ts_bad\tyes\tnot-base64\n"
        );
        let rows = parse_list(&stdout).unwrap_or_else(|failure| panic!("{failure:?}"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "s_a");
        assert!(rows[0].alive && rows[0].meta.name == "alpha");
        assert_eq!(rows[1].id, "s_b");
        assert!(!rows[1].alive);
        assert_eq!(
            parse_list("RHOST_ERR=notmux\n").err(),
            Some(HelperFailure::NoTmux)
        );
    }

    fn token() -> String {
        "a".repeat(32)
    }

    #[test]
    fn an_exec_result_needs_this_invocations_token_and_one_status() {
        let good = format!(
            "RHOST_ID=s_ab\nRHOST_TOKEN={}\nRHOST_EXIT=7\nRHOST_OUTPUT={}\n",
            token(),
            base64::encode(b"out\n")
        );
        match parse_exec(&good, &token()) {
            ExecResult::Completed {
                id,
                output,
                exit_code,
            } => {
                assert_eq!(id, "s_ab");
                assert_eq!(output, b"out\n");
                assert_eq!(exit_code, 7);
            }
            other => panic!("{other:?}"),
        }
        for broken in [
            // Another invocation's token, a duplicated status, a status written
            // differently, a missing identity, a stray line, a duplicate identity.
            format!(
                "RHOST_ID=s_ab\nRHOST_TOKEN={}\nRHOST_EXIT=0\nRHOST_OUTPUT=\n",
                "b".repeat(32)
            ),
            format!(
                "RHOST_ID=s_ab\nRHOST_TOKEN={0}\nRHOST_EXIT=0\nRHOST_EXIT=1\nRHOST_OUTPUT=\n",
                token()
            ),
            format!(
                "RHOST_ID=s_ab\nRHOST_TOKEN={}\nRHOST_EXIT=07\nRHOST_OUTPUT=\n",
                token()
            ),
            format!("RHOST_TOKEN={}\nRHOST_EXIT=0\nRHOST_OUTPUT=\n", token()),
            format!(
                "RHOST_ID=s_ab\nRHOST_TOKEN={}\nRHOST_EXIT=0\nRHOST_OUTPUT=\nnoise\n",
                token()
            ),
            format!(
                "RHOST_ID=s_ab\nRHOST_ID=s_cd\nRHOST_TOKEN={}\nRHOST_EXIT=0\nRHOST_OUTPUT=\n",
                token()
            ),
        ] {
            assert!(
                matches!(
                    parse_exec(&broken, &token()),
                    ExecResult::Failed {
                        failure: HelperFailure::Protocol,
                        ..
                    }
                ),
                "{broken}"
            );
        }
    }

    #[test]
    fn a_refusal_carries_what_owns_the_pane_and_whether_the_shell_came_back() {
        match parse_exec("RHOST_ID=s_ab\nRHOST_FG=cat\nRHOST_ERR=busy\n", &token()) {
            ExecResult::Failed {
                failure,
                id,
                foreground,
                recovered,
            } => {
                assert_eq!(failure, HelperFailure::Busy);
                assert_eq!(id.as_deref(), Some("s_ab"));
                assert_eq!(foreground.as_deref(), Some("cat"));
                assert!(!recovered);
            }
            other => panic!("{other:?}"),
        }
        match parse_exec(
            "RHOST_ID=s_ab\nRHOST_RECOVERED=1\nRHOST_ERR=timeout\n",
            &token(),
        ) {
            ExecResult::Failed {
                failure, recovered, ..
            } => {
                assert_eq!(failure, HelperFailure::Timeout);
                assert!(recovered);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_read_needs_consistent_cursors_and_decodes_its_page() {
        let ok = format!(
            "RHOST_ID=s_ab\nRHOST_FROM=4\nRHOST_NEXT=9\nRHOST_SIZE=20\n{}\n",
            base64::encode(b"hello")
        );
        let read = parse_read(&ok).unwrap_or_else(|failure| panic!("{failure:?}"));
        assert_eq!((read.from, read.next, read.size), (4, 9, 20));
        assert_eq!(read.data, b"hello");
        for broken in [
            // A cursor that goes backwards, a size smaller than the cursor, a
            // missing field, a duplicated one, an unknown field, no identity.
            "RHOST_ID=s_ab\nRHOST_FROM=9\nRHOST_NEXT=4\nRHOST_SIZE=20\n\n",
            "RHOST_ID=s_ab\nRHOST_FROM=0\nRHOST_NEXT=9\nRHOST_SIZE=4\n\n",
            "RHOST_ID=s_ab\nRHOST_FROM=0\nRHOST_SIZE=4\n\n",
            "RHOST_ID=s_ab\nRHOST_FROM=0\nRHOST_FROM=0\nRHOST_NEXT=0\nRHOST_SIZE=0\n\n",
            "RHOST_ID=s_ab\nRHOST_FROM=0\nRHOST_NEXT=0\nRHOST_SIZE=0\nRHOST_WHAT=1\n\n",
            "RHOST_FROM=0\nRHOST_NEXT=0\nRHOST_SIZE=0\n\n",
        ] {
            assert_eq!(
                parse_read(broken).err(),
                Some(HelperFailure::Protocol),
                "{broken}"
            );
        }
        assert_eq!(
            parse_read("RHOST_ERR=nosession\n").err(),
            Some(HelperFailure::NoSession)
        );
    }
}
