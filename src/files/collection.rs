//! Immutable path discovery with fresh, bounded content observations per page.
use super::RequestOutcome;
use crate::{
    domain::{Phase, Recovery, Submission},
    remote::Error,
    transport::Client,
};
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
    #[serde(default)]
    exclude_dirs: Vec<String>,
    #[serde(default = "ttl")]
    collection_ttl_seconds: u64,
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
fn ttl() -> u64 {
    600
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
fn nonnull<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
fn invalid() -> Error {
    Error::new("CONFIG_INVALID", "invalid structured discovery request")
        .with_recovery(Recovery::invalid())
}
fn bad() -> Error {
    Error::new(
        "PROTOCOL_ERROR",
        "invalid discovered-path collection answer",
    )
}
fn sha(s: &str) -> bool {
    crate::domain::Sha256::new(s.into()).is_ok()
}
fn path(s: &str) -> bool {
    super::backend::validate_remote_path(s).is_ok()
}
fn cursor(s: &str) -> Result<(&str, &str, u64), Error> {
    let parts: Vec<_> = s.split(':').collect();
    if parts.len() != 3
        || !crate::helper::registry::generation(parts[0])
        || crate::helper::registry::key(parts[1]).is_none()
        || parts[2].is_empty()
        || parts[2].len() > 8
        || !parts[2].bytes().all(|c| c.is_ascii_digit())
        || (parts[2].starts_with('0') && parts[2].len() > 1)
    {
        return Err(invalid());
    }
    let offset = parts[2].parse().map_err(|_| invalid())?;
    if offset > 8388608 {
        return Err(invalid());
    }
    Ok((parts[0], parts[1], offset))
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
struct Found {
    path: String,
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
struct Collection {
    generation: String,
    id: String,
    expires_unix: u64,
    query_sha256: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Page<T> {
    root: String,
    collection: Collection,
    items: Vec<T>,
    skipped: Vec<Skipped>,
    scanned_files: u64,
    scanned_bytes: u64,
    incomplete: bool,
    reasons: Vec<Reason>,
    #[serde(deserialize_with = "required")]
    next_cursor: Option<String>,
}
fn check<T>(r: &Page<T>, p: &Search) -> Result<(), Error> {
    if !path(&r.root)
        || !crate::helper::registry::generation(&r.collection.generation)
        || crate::helper::registry::key(&r.collection.id) != Some(r.collection.expires_unix)
        || !sha(&r.collection.query_sha256)
        || r.items.len() > p.max_results
        || r.skipped.len() > 128
        || r.scanned_files > 100000
        || r.scanned_bytes > 67108864
        || r.incomplete == r.reasons.is_empty()
        || r.skipped
            .iter()
            .any(|s| !path(&s.path) || super::backend::known_refusal_code(&s.code).is_none())
    {
        return Err(bad());
    }
    if let Some(ref c) = p.cursor {
        let (g, id, _) = cursor(c)?;
        if g != r.collection.generation || id != r.collection.id {
            return Err(bad());
        }
    }
    if let Some(ref c) = r.next_cursor {
        let (g, id, offset) = cursor(c).map_err(|_| bad())?;
        if g != r.collection.generation
            || id != r.collection.id
            || p.cursor
                .as_deref()
                .is_some_and(|old| cursor(old).is_ok_and(|(_, _, prior)| offset <= prior))
        {
            return Err(bad());
        }
    }
    Ok(())
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
        let original: Value = serde_json::from_str(raw).map_err(|_| invalid())?;
        let p: Search = serde_json::from_str(raw).map_err(|_| invalid())?;
        let find = operation == "fs.find";
        if !path(&p.root)
            || p.glob.is_empty()
            || p.glob.len() > 4096
            || p.glob.contains('\0')
            || p.pattern
                .as_deref()
                .is_some_and(|v| v.is_empty() || v.len() > 4096 || v.contains(['\0', '\n', '\r']))
            || !(1..=128).contains(&p.max_results)
            || !(1..=512).contains(&p.max_matches)
            || !(1..=1048576).contains(&p.max_bytes)
            || !(1..=3600).contains(&p.collection_ttl_seconds)
            || p.exclude_dirs.len() > 64
            || p.exclude_dirs.iter().any(|s| {
                s.is_empty()
                    || s.len() > 4096
                    || s.starts_with('/')
                    || s.contains('\0')
                    || s.split('/').any(|c| matches!(c, "" | "." | ".."))
            })
            || p.exclude_dirs
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != p.exclude_dirs.len()
            || (find
                && (original.get("pattern").is_some() || original.get("max_matches").is_some()))
        {
            return Err(invalid());
        }
        if let Some(ref c) = p.cursor {
            cursor(c)?;
        }
        let mut q = json!({"op":if find {"find"}else{"search"},"root":p.root,"glob":p.glob,"exclude_dirs":p.exclude_dirs,"collection_ttl_seconds":p.collection_ttl_seconds,"max_results":p.max_results,"max_bytes":p.max_bytes});
        if !find {
            q["max_matches"] = p.max_matches.into();
        }
        if let Some(ref pattern) = p.pattern {
            q["pattern"] = pattern.clone().into();
        }
        if let Some(ref c) = p.cursor {
            q["cursor"] = c.clone().into();
        }
        if find {
            let r: Page<Found> =
                super::inspect::call(client, host, q, 16 * 1024 * 1024, cancelled, guard)?;
            check(&r, &p)?;
            if r.scanned_bytes != 0 || r.items.iter().any(|f| !path(&f.path)) {
                return Err(bad());
            }
            serde_json::to_value(r).map_err(|_| bad())
        } else {
            let r: Page<File> =
                super::inspect::call(client, host, q, 16 * 1024 * 1024, cancelled, guard)?;
            check(&r, &p)?;
            if r.items.iter().map(|f| f.matches.len()).sum::<usize>() > p.max_matches
                || r.items
                    .iter()
                    .flat_map(|f| &f.matches)
                    .map(|m| m.text.len())
                    .sum::<usize>()
                    > p.max_bytes
                || r.items.iter().any(|f| {
                    !sha(&f.sha256)
                        || !path(&f.path)
                        || f.bytes > 8388608
                        || f.matches.iter().any(|m| m.line == 0)
                })
            {
                return Err(bad());
            }
            serde_json::to_value(r).map_err(|_| bad())
        }
    })();
    match result {
        Ok(v) => RequestOutcome::done(v),
        Err(mut e) => {
            e.mutation = None;
            if e.recovery.submission() != "not_submitted" {
                e.recovery = Recovery::observe(
                    Phase::Observation,
                    if e.recovery.submission() == "confirmed" {
                        Submission::Confirmed
                    } else {
                        Submission::MayHaveSubmitted
                    },
                );
            }
            RequestOutcome::failed(e)
        }
    }
}
