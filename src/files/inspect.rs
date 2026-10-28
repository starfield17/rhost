//! Structured, bounded observations; revisions are not workspace snapshots.
use super::{
    RequestOutcome,
    backend::{Answer, ReadResult},
};
use crate::{remote::Error, transport::Client};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    root: String,
    #[serde(default = "glob")]
    glob: String,
    #[serde(default, deserialize_with = "nonnull")]
    pattern: Option<String>,
    #[serde(default, deserialize_with = "nonnull")]
    cursor: Option<String>,
    #[serde(default = "results")]
    max_results: usize,
    #[serde(default = "matches")]
    max_matches: usize,
    #[serde(default = "cap")]
    max_bytes: usize,
}
fn glob() -> String {
    "*".into()
}
fn results() -> usize {
    64
}
fn matches() -> usize {
    128
}
fn cap() -> usize {
    262144
}
fn one() -> u64 {
    1
}
fn lines() -> u64 {
    200
}
fn nonnull<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Many {
    paths: Vec<String>,
    #[serde(default = "one")]
    start: u64,
    #[serde(default = "lines")]
    lines: u64,
    #[serde(default = "cap")]
    max_bytes: usize,
}
fn parse<T: for<'a> Deserialize<'a>>(raw: &str) -> Result<T, Error> {
    serde_json::from_str(raw).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))
}
fn invalid() -> Error {
    Error::new("CONFIG_INVALID", "invalid structured inspection bounds")
}
fn bad() -> Error {
    Error::new("PROTOCOL_ERROR", "invalid structured file observation")
}
fn path(s: &str) -> Result<(), Error> {
    super::backend::validate_remote_path(s).map_err(|_| invalid())
}
fn sha(s: &str) -> Result<(), Error> {
    crate::domain::Sha256::new(s.into())
        .map(|_| ())
        .map_err(|_| bad())
}
fn cursor(s: &str) -> Result<(&str, usize), Error> {
    let (revision, offset) = s.split_once(':').ok_or_else(invalid)?;
    sha(revision).map_err(|_| invalid())?;
    if offset.is_empty() || offset.len() > 4 || !offset.bytes().all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }
    let n = offset.parse().map_err(|_| invalid())?;
    if n > 4096 {
        return Err(invalid());
    }
    Ok((revision, n))
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Match {
    line: u64,
    text: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct File {
    path: String,
    sha256: String,
    bytes: u64,
    matches: Vec<Match>,
    matches_truncated: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Skipped {
    path: String,
    code: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    EntryLimit,
    DepthLimit,
    ScanByteLimit,
    FileLimit,
    Unreadable,
    InvalidText,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SearchResult {
    root: String,
    revision: String,
    items: Vec<File>,
    skipped: Vec<Skipped>,
    scanned_files: u64,
    scanned_bytes: u64,
    incomplete: bool,
    reasons: Vec<Reason>,
    #[serde(deserialize_with = "required")]
    next_cursor: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Detail {
    code: String,
    message: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Item {
    path: String,
    ok: bool,
    #[serde(deserialize_with = "required")]
    data: Option<ReadResult>,
    #[serde(deserialize_with = "required")]
    error: Option<Detail>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManyResult {
    items: Vec<Item>,
    unread: Vec<String>,
}
fn call<T: for<'a> Deserialize<'a>>(
    client: &Client,
    host: &str,
    payload: Value,
    limit: usize,
    cancelled: &dyn Fn() -> bool,
    guard: Option<&crate::domain::BootId>,
) -> Result<T, Error> {
    let bytes = serde_json::to_vec(&payload).map_err(|_| invalid())?;
    if bytes.len() > 12 * 1024 * 1024 {
        return Err(invalid());
    }
    let reply = crate::helper::Helper::new(client, host)
        .with_cancellation(cancelled)
        .with_boot_guard(guard)
        .file_call(&bytes, std::time::Duration::from_secs(60), limit)?;
    match serde_json::from_slice::<Answer<T>>(&reply).map_err(|_| bad())? {
        Answer::Done(v) => Ok(v),
        Answer::Refused(e) => Err(super::request::refusal(e)),
    }
}
pub(super) fn execute(
    client: &Client,
    host: &str,
    operation: &str,
    raw: &str,
    cancelled: &dyn Fn() -> bool,
    guard: Option<&crate::domain::BootId>,
) -> RequestOutcome {
    let result = (|| {
        if operation == "fs.search" {
            let p: Search = parse(raw)?;
            path(&p.root)?;
            if p.glob.is_empty()
                || p.glob.len() > 4096
                || p.glob.contains('\0')
                || p.pattern
                    .as_deref()
                    .is_some_and(|v| v.is_empty() || v.len() > 4096 || v.contains(['\n', '\r']))
                || !(1..=128).contains(&p.max_results)
                || !(1..=512).contains(&p.max_matches)
                || !(1..=1048576).contains(&p.max_bytes)
            {
                return Err(invalid());
            }
            if let Some(ref c) = p.cursor {
                cursor(c)?;
            }
            let mut q = json!({"op":"search","root":p.root,"glob":p.glob,"max_results":p.max_results,"max_matches":p.max_matches,"max_bytes":p.max_bytes});
            if let Some(ref pattern) = p.pattern {
                q["pattern"] = pattern.clone().into();
            }
            if let Some(ref c) = p.cursor {
                q["cursor"] = c.clone().into();
            }
            let r: SearchResult = call(client, host, q, 16 * 1024 * 1024, cancelled, guard)?;
            sha(&r.revision)?;
            let count = r.items.iter().map(|f| f.matches.len()).sum::<usize>();
            let bytes = r
                .items
                .iter()
                .flat_map(|f| &f.matches)
                .map(|m| m.text.len())
                .sum::<usize>();
            if r.items.len() > p.max_results
                || count > p.max_matches
                || bytes > p.max_bytes
                || r.skipped.len() > 128
                || r.scanned_files > 4096
                || r.scanned_bytes > 67108864
                || r.incomplete == r.reasons.is_empty()
                || (r.incomplete && r.next_cursor.is_some())
            {
                return Err(bad());
            }
            if let Some(ref c) = r.next_cursor {
                let (revision, _) = cursor(c).map_err(|_| bad())?;
                if revision != r.revision {
                    return Err(bad());
                }
            }
            for f in &r.items {
                sha(&f.sha256)?;
                path(&f.path).map_err(|_| bad())?;
                if f.bytes > 8388608 || f.matches.iter().any(|m| m.line == 0) {
                    return Err(bad());
                }
            }
            for s in &r.skipped {
                if super::backend::known_refusal_code(&s.code).is_none() {
                    return Err(bad());
                }
            }
            serde_json::to_value(r).map_err(|_| bad())
        } else {
            let p: Many = parse(raw)?;
            if p.paths.is_empty()
                || p.paths.len() > 64
                || p.start == 0
                || p.lines == 0
                || !(1..=8388608).contains(&p.max_bytes)
            {
                return Err(invalid());
            }
            for name in &p.paths {
                path(name)?;
            }
            let r: ManyResult = call(
                client,
                host,
                json!({"op":"read_many","paths":p.paths,"start":p.start,"lines":p.lines,"max_bytes":p.max_bytes}),
                64 * 1024 * 1024,
                cancelled,
                guard,
            )?;
            if r.items.len() + r.unread.len() != p.paths.len() {
                return Err(bad());
            }
            let mut bytes = 0;
            for item in &r.items {
                match (&item.data, &item.error, item.ok) {
                    (Some(data), None, true) => {
                        sha(&data.sha256)?;
                        if data.path != item.path
                            || data.start != p.start
                            || data.lines > p.lines
                            || data.lines > data.total_lines
                        {
                            return Err(bad());
                        }
                        bytes += data.content.len();
                    }
                    (None, Some(error), false) => {
                        if super::backend::known_refusal_code(&error.code).is_none() {
                            return Err(bad());
                        }
                    }
                    _ => return Err(bad()),
                }
            }
            if bytes > p.max_bytes {
                return Err(bad());
            }
            serde_json::to_value(r).map_err(|_| bad())
        }
    })();
    match result {
        Ok(data) => RequestOutcome::done(data),
        Err(mut error) => {
            error.mutation = None;
            RequestOutcome::failed(error)
        }
    }
}
