//! The typed remote filesystem-helper protocol and its one-shot bootstrap.
//!
//! One JSON request goes in, one JSON answer comes out. The program is embedded
//! in the binary and executed by the *remote* python3; nothing is installed
//! there (AGENTS.md §5). It travels as one quoted argument, so no path or file
//! body ever has to survive a shell parser, and the request goes in on stdin for
//! the same reason.
//!
//! Why an embedded interpreter script rather than Rust or a generated shell
//! program, given the session helpers are shell scripts:
//!
//! * the helper runs on a host whose architecture rhost does not know (x86_64,
//!   aarch64, ...), so a Rust helper would have to be cross-compiled per target;
//!   `check-release-contract.sh` refuses a cross-compiler, and rhost never
//!   installs anything remotely (AGENTS.md §5), so a native remote binary is out;
//! * unlike the session helpers, this one has to move binary bodies and return
//!   structured bytes: `fs write`/`patch` carry base64 content and hashes in a
//!   JSON request and must reply with one bounded JSON document, under a `flock`
//!   and an atomic same-directory replace — logic that POSIX shell plus
//!   coreutils cannot express without inventing a fragile quoting protocol;
//! * an already-present `python3` on the target gives exactly that JSON pump
//!   with no install step, and a host without it is reported as
//!   `REMOTE_DEPENDENCY_MISSING`, never as a silent failure.
//!
//! `docs/CONTRACT.md` explicitly leaves the helper's source language unfrozen, so
//! this can change later if a target without python3 ever justifies it. The
//! *protocol* is the stable part: the types below are what both halves of one
//! rhost build agree the request and answer look like.

use serde::{Deserialize, Serialize};

/// The embedded helper. Its refusal codes are the CLI's own taxonomy, which is
/// why the caller may trust a code but never an arbitrary message.
pub const PROGRAM: &str = include_str!("remote_fs.py");

/// A successful edit answer is small, so this is a ceiling rather than a request
/// to read a whole file. `fs patch` returns the replacement's hash and length.
pub const DEFAULT_HELPER_BYTES: usize = 256 * 1024;

/// The remote command that runs the helper. A host without python3 exits 127,
/// which the caller reports as a missing dependency rather than a tool failure.
pub fn command() -> String {
    format!(
        "command -v python3 >/dev/null 2>&1 || exit 127; python3 -c {}",
        crate::shell::quote(PROGRAM)
    )
}

/// The transport budget for one helper answer: JSON may escape every ASCII
/// control byte as six bytes, and the document around the payload needs room.
pub fn output_budget(max_bytes: usize) -> usize {
    max_bytes.saturating_mul(6).saturating_add(64 * 1024)
}

/// One line-range replacement in the request's patch document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub start: u64,
    pub end: u64,
    pub text: String,
}

/// Every request shape the embedded helper understands.
#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request<'a> {
    Resolve {
        path: &'a str,
        delete: bool,
    },
    Read {
        path: &'a str,
        start: u64,
        lines: u64,
        max_bytes: usize,
    },
    Write {
        path: &'a str,
        content: String,
        if_hash: &'a str,
        parents: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_mode: Option<&'a str>,
        max_bytes: usize,
    },
    Patch {
        path: &'a str,
        edits: Vec<Edit>,
        if_hash: &'a str,
        max_bytes: usize,
    },
}

/// One decoded `fs read` answer. Every field is required: a partial answer is
/// not a usable page, and guessing the missing part is how a caller ends up
/// acting on content it never saw.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadResult {
    pub path: String,
    pub sha256: String,
    pub content: String,
    pub start: u64,
    pub lines: u64,
    pub total_lines: u64,
    pub truncated: bool,
}

/// One decoded `fs write` or `fs patch` answer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteResult {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// One decoded destructive-sync target answer.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveResult {
    pub path: String,
}

/// Either the operation's typed success document or a structured refusal.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Answer<T> {
    Done(T),
    Refused(HelperRefusal),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperRefusal {
    pub(crate) error: String,
    #[serde(default)]
    pub(crate) message: Option<String>,
}

impl HelperRefusal {
    pub(crate) fn message(&self) -> &str {
        self.message
            .as_deref()
            .unwrap_or("remote file operation was refused")
    }
}

/// The refusal codes the helper may use. Anything else is the remote inventing
/// an error, which the caller reports as `INTERNAL` rather than a code an agent
/// might branch on.
pub fn known_refusal_code(code: &str) -> Option<&'static str> {
    const CODES: [&str; 12] = [
        "CONFIG_INVALID",
        "FILE_CONFLICT",
        "HASH_REQUIRED",
        "FILE_NOT_FOUND",
        "FILE_TOO_LARGE",
        "INVALID_PATCH",
        "INVALID_TARGET",
        "INVALID_TEXT",
        "SYNC_REJECTED",
        "REMOTE_DEPENDENCY_MISSING",
        "INTERNAL",
        "USAGE_ERROR",
    ];
    CODES
        .into_iter()
        .find(|known| known.eq_ignore_ascii_case(code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base64;

    #[test]
    fn the_helper_travels_as_one_quoted_argument() {
        let command = command();
        assert!(command.starts_with("command -v python3"), "{command}");
        assert!(command.contains("python3 -c '"), "{command}");
        // The program is quoted, not escaped: whatever it contains — newlines
        // included — stays one argv element for the remote shell.
        assert!(command.trim_end().ends_with('\''), "{command}");
        assert!(
            command.contains("CONFIG_INVALID"),
            "the program is embedded"
        );
    }

    #[test]
    fn a_missing_interpreter_is_a_dependency_failure_not_a_silent_run() {
        let command = command();
        // The probe must gate execution: no python3 means exit 127 (the code the
        // caller maps to REMOTE_DEPENDENCY_MISSING), before any helper logic.
        assert!(
            command.contains("command -v python3 >/dev/null 2>&1 || exit 127;"),
            "the helper must refuse to run without python3: {command}"
        );
        // The program is compiled in, never read from a remote path.
        assert!(
            PROGRAM.contains("import json") && PROGRAM.contains("def run("),
            "the embedded program looks truncated"
        );
        assert!(
            !command.contains('/') || command.contains("python3 -c"),
            "the helper must be embedded, not a remote path: {command}"
        );
    }

    #[test]
    fn requests_have_the_helper_protocol_shape() -> Result<(), serde_json::Error> {
        let read = serde_json::to_value(Request::Read {
            path: "~/file",
            start: 2,
            lines: 3,
            max_bytes: 4096,
        })?;
        assert_eq!(
            read,
            serde_json::json!({
                "op": "read",
                "path": "~/file",
                "start": 2,
                "lines": 3,
                "max_bytes": 4096
            })
        );

        let write = serde_json::to_value(Request::Write {
            path: "/srv/file",
            content: base64::encode(b"hello\n"),
            if_hash: "a".repeat(64).as_str(),
            parents: true,
            file_mode: Some("0640"),
            max_bytes: DEFAULT_HELPER_BYTES,
        })?;
        assert_eq!(
            write["content"],
            serde_json::json!("aGVsbG8K"),
            "content travels as standard base64"
        );
        assert_eq!(write["file_mode"], serde_json::json!("0640"));
        assert_eq!(write["max_bytes"], serde_json::json!(DEFAULT_HELPER_BYTES));

        let without_mode = serde_json::to_value(Request::Write {
            path: "/srv/file",
            content: base64::encode(b"hello\n"),
            if_hash: "",
            parents: false,
            file_mode: None,
            max_bytes: DEFAULT_HELPER_BYTES,
        })?;
        assert!(
            without_mode.get("file_mode").is_none(),
            "an unnamed mode must not invent one"
        );

        let patch = serde_json::to_value(Request::Patch {
            path: "/srv/file",
            edits: vec![Edit {
                start: 2,
                end: 2,
                text: "replacement\n".to_string(),
            }],
            if_hash: "b".repeat(64).as_str(),
            max_bytes: DEFAULT_HELPER_BYTES,
        })?;
        assert_eq!(patch["op"], serde_json::json!("patch"));
        assert_eq!(
            patch["edits"],
            serde_json::json!([{"start": 2, "end": 2, "text": "replacement\n"}])
        );
        Ok(())
    }

    #[test]
    fn answers_are_typed_before_the_caller_uses_them() -> Result<(), serde_json::Error> {
        let read: Answer<ReadResult> = serde_json::from_str(
            r#"{"path":"/srv/file","sha256":"a","content":"x\n","start":1,"lines":1,"total_lines":1,"truncated":false}"#,
        )?;
        match read {
            Answer::Done(result) => {
                assert_eq!(result.path, "/srv/file");
                assert_eq!(result.total_lines, 1);
            }
            Answer::Refused(refusal) => panic!("a success answer decoded as {refusal:?}"),
        }

        let refusal: Answer<ReadResult> =
            serde_json::from_str(r#"{"error":"FILE_NOT_FOUND","message":"gone"}"#)?;
        match refusal {
            Answer::Refused(refusal) => {
                assert_eq!(known_refusal_code(&refusal.error), Some("FILE_NOT_FOUND"));
                assert_eq!(refusal.message(), "gone");
            }
            Answer::Done(_) => panic!("a refusal decoded as success"),
        }

        assert!(
            serde_json::from_str::<Answer<ReadResult>>(
                r#"{"path":"/srv/file","sha256":"a","content":"x\n","start":1,"lines":1,"total_lines":1,"truncated":false,"surprise":true}"#,
            )
            .is_err(),
            "a helper answer with an unknown field is protocol drift"
        );
        assert_eq!(known_refusal_code("INVENTED_ERROR"), None);
        Ok(())
    }
}
