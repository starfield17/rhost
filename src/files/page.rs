//! Validate source coverage before a page reaches any delivery adapter.
use crate::remote::Error;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Coverage {
    from: u64,
    next: u64,
    observed_bytes: u64,
    more: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Bytes {
    requested_path: String,
    path: String,
    sha256: String,
    content: String,
    coverage: Coverage,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Lines {
    requested_path: String,
    path: String,
    sha256: String,
    content: String,
    start: u64,
    lines: u64,
    total_lines: u64,
    truncated: bool,
    coverage: Coverage,
}
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum Page {
    Lines(Lines),
    Bytes(Bytes),
}
fn bad() -> Error {
    Error::new("PROTOCOL_ERROR", "inconsistent file page evidence")
}
pub(super) fn validate(
    value: Value,
    path: &str,
    cap: usize,
    window: Option<(u64, u64)>,
) -> Result<Value, Error> {
    let p: Page = serde_json::from_value(value).map_err(|_| bad())?;
    let (name, requested, hash, text, c) = match &p {
        Page::Bytes(v) => (
            &v.path,
            &v.requested_path,
            &v.sha256,
            &v.content,
            &v.coverage,
        ),
        Page::Lines(v) => {
            if v.start == 0
                || v.lines > v.total_lines
                || window.is_some_and(|(s, n)| v.start != s || v.lines > n)
            {
                return Err(bad());
            }
            (
                &v.path,
                &v.requested_path,
                &v.sha256,
                &v.content,
                &v.coverage,
            )
        }
    };
    if requested != path
        || !name.starts_with('/')
        || crate::domain::Sha256::new(hash.clone()).is_err()
        || c.next < c.from
        || c.next - c.from != text.len() as u64
        || text.len() > cap
        || c.next > c.observed_bytes
        || c.more != (c.next < c.observed_bytes)
    {
        return Err(bad());
    }
    let next = if c.more {
        Some(
            json!({"operation":"fs.read","params":{"path":name,"offset_bytes":c.next,"if_hash":hash,"max_bytes":cap.max(4)}}),
        )
    } else {
        None
    };
    let mut value = serde_json::to_value(p).map_err(|_| bad())?;
    value["coverage"]["next_request"] = json!(next);
    Ok(value)
}
