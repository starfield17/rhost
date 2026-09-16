//! The remote file helper: one JSON request in, one JSON answer out.
//!
//! The program is embedded in the binary and executed by the *remote* python3;
//! nothing is installed there (AGENTS.md §5). It travels as one quoted argument,
//! so no path or file body ever has to survive a shell parser, and the request
//! goes in on stdin for the same reason.
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
//! this can change later if a target without python3 ever justifies it.

use crate::shell;
use serde_json::{Map, Value, json};

/// The embedded helper. Its refusal codes are the CLI's own taxonomy, which is
/// why the caller may trust a code but never an arbitrary message.
pub const PROGRAM: &str = include_str!("remote_edit.py");

/// The remote command that runs the helper. A host without python3 exits 127,
/// which the caller reports as a missing dependency rather than a tool failure.
pub fn command() -> String {
    format!(
        "command -v python3 >/dev/null 2>&1 || exit 127; python3 -c {}",
        shell::quote(PROGRAM)
    )
}

/// The transport budget for one helper answer: JSON may escape every ASCII
/// control byte as six bytes, and the document around the payload needs room.
pub fn output_budget(max_bytes: usize) -> usize {
    max_bytes.saturating_mul(6).saturating_add(64 * 1024)
}

pub fn resolve_request(path: &str, delete: bool) -> Value {
    let mut request = Map::new();
    request.insert("op".into(), json!("resolve"));
    request.insert("path".into(), json!(path));
    request.insert("delete".into(), json!(delete));
    Value::Object(request)
}

pub fn read_request(path: &str, start: u64, lines: u64, max_bytes: usize) -> Value {
    let mut request = Map::new();
    request.insert("op".into(), json!("read"));
    request.insert("path".into(), json!(path));
    request.insert("start".into(), json!(start));
    request.insert("lines".into(), json!(lines));
    request.insert("max_bytes".into(), json!(max_bytes));
    Value::Object(request)
}

pub fn write_request(
    path: &str,
    content: &[u8],
    if_hash: &str,
    parents: bool,
    mode: Option<&str>,
) -> Value {
    let mut request = Map::new();
    request.insert("op".into(), json!("write"));
    request.insert("path".into(), json!(path));
    request.insert("content".into(), json!(base64(content)));
    request.insert("if_hash".into(), json!(if_hash));
    request.insert("parents".into(), json!(parents));
    if let Some(mode) = mode {
        request.insert("file_mode".into(), json!(mode));
    }
    request.insert("max_bytes".into(), json!(256 * 1024));
    Value::Object(request)
}

pub fn patch_request(path: &str, edits: Vec<Value>, if_hash: &str, max_bytes: usize) -> Value {
    let mut request = Map::new();
    request.insert("op".into(), json!("patch"));
    request.insert("path".into(), json!(path));
    request.insert("edits".into(), Value::Array(edits));
    request.insert("if_hash".into(), json!(if_hash));
    request.insert("max_bytes".into(), json!(max_bytes));
    Value::Object(request)
}

/// One decoded `fs read` answer. Every field is required: a partial answer is
/// not a usable page, and guessing the missing part is how a caller ends up
/// acting on content it never saw.
pub struct ReadResult {
    pub path: String,
    pub sha256: String,
    pub content: String,
    pub start: u64,
    pub lines: u64,
    pub total_lines: u64,
    pub truncated: bool,
}

pub struct WriteResult {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

fn text(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("helper answer has no {key}"))
}

fn count(value: &Value, key: &str) -> Result<u64, String> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("helper answer has no {key}"))
}

pub fn read_result(value: &Value) -> Result<ReadResult, String> {
    Ok(ReadResult {
        path: text(value, "path")?,
        sha256: text(value, "sha256")?,
        content: text(value, "content")?,
        start: count(value, "start")?,
        lines: count(value, "lines")?,
        total_lines: count(value, "total_lines")?,
        truncated: value
            .get("truncated")
            .and_then(Value::as_bool)
            .ok_or("helper answer has no truncated")?,
    })
}

pub fn write_result(value: &Value) -> Result<WriteResult, String> {
    Ok(WriteResult {
        path: text(value, "path")?,
        sha256: text(value, "sha256")?,
        bytes: count(value, "bytes")?,
    })
}

pub fn resolved_path(value: &Value) -> Result<String, String> {
    text(value, "path")
}

/// The refusal a helper answer carries, if it carries one.
pub fn refusal(value: &Value) -> Option<(String, String)> {
    let code = value.get("error").and_then(Value::as_str)?;
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("remote file operation was refused")
        .to_string();
    Some((code.to_string(), message))
}

/// Standard base64, written here rather than pulled in: the helper's request
/// format is the only place the crate needs it, and an encoder is sixteen lines
/// of table.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let index = ((block[0] as u32) << 16) | ((block[1] as u32) << 8) | block[2] as u32;
        for position in 0..4 {
            if position <= chunk.len() {
                let shift = 18 - 6 * position;
                out.push(ALPHABET[((index >> shift) & 0x3F) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64("héllo\n".as_bytes()), "aMOpbGxvCg==");
    }

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
}
