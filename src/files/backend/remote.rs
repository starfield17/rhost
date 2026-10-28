//! Typed one-shot native filesystem requests; shell syntax never carries file bodies.

use serde::{Deserialize, Serialize};

/// A successful edit answer is small; bounded reads reserve their own budget.
pub const DEFAULT_HELPER_BYTES: usize = 256 * 1024;

/// The transport budget for one helper answer: JSON may escape every ASCII
/// control byte as six bytes, and the document around the payload needs room.
pub fn output_budget(max_bytes: usize) -> usize {
    max_bytes.saturating_mul(6).saturating_add(64 * 1024)
}

/// One line-range or exact-text replacement in the request's patch document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Edit {
    Range(RangeEdit),
    Replace(ReplaceEdit),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RangeEdit {
    pub start: u64,
    pub end: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplaceEdit {
    pub old: String,
    pub new: String,
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
#[derive(Debug, Serialize, Deserialize)]
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
    #[serde(default)]
    pub(crate) mutation: Option<String>,
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
    const CODES: [&str; 18] = [
        "NAMESPACE_MISMATCH",
        "IDEMPOTENCY_CONFLICT",
        "REQUEST_EXPIRED",
        "CONTEXT_CHANGED",
        "CONTEXT_UNAVAILABLE",
        "STATE_UNAVAILABLE",
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
    fn file_requests_preserve_paths_and_bodies_as_data() {
        let path = "/srv/file with 'quotes';$(command)\nnext";
        let request = Request::Write {
            path,
            content: base64::encode(b"a\0b\n"),
            if_hash: "",
            parents: false,
            file_mode: None,
            max_bytes: 4096,
        };
        let encoded =
            serde_json::to_vec(&request).unwrap_or_else(|e| panic!("encode request: {e}"));
        let decoded: serde_json::Value =
            serde_json::from_slice(&encoded).unwrap_or_else(|e| panic!("decode request: {e}"));
        assert_eq!(decoded["path"], path);
        assert_eq!(decoded["content"], "YQBiCg==");
        assert_eq!(decoded["if_hash"], "");
        assert!(decoded.get("file_mode").is_none());
    }

    #[test]
    fn escaped_response_bounds_cover_control_bytes_without_overflow() {
        let text: String = (0..32).map(char::from).collect();
        let encoded = serde_json::to_vec(&serde_json::json!({"content": text}))
            .unwrap_or_else(|e| panic!("encode controls: {e}"));
        assert!(encoded.len() <= output_budget(32));
        assert_eq!(output_budget(0), 65536);
        assert_eq!(output_budget(8388608), 50397184);
        assert_eq!(output_budget(usize::MAX), usize::MAX);
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
            edits: vec![Edit::Range(RangeEdit {
                start: 2,
                end: 2,
                text: "replacement\n".to_string(),
            })],
            if_hash: "b".repeat(64).as_str(),
            max_bytes: DEFAULT_HELPER_BYTES,
        })?;
        assert_eq!(patch["op"], serde_json::json!("patch"));
        assert_eq!(
            patch["edits"],
            serde_json::json!([{"start": 2, "end": 2, "text": "replacement\n"}])
        );

        let patch_replace = serde_json::to_value(Request::Patch {
            path: "/srv/file",
            edits: vec![Edit::Replace(ReplaceEdit {
                old: "foo\n".to_string(),
                new: "bar\n".to_string(),
            })],
            if_hash: "b".repeat(64).as_str(),
            max_bytes: DEFAULT_HELPER_BYTES,
        })?;
        assert_eq!(
            patch_replace["edits"],
            serde_json::json!([{"old": "foo\n", "new": "bar\n"}])
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
