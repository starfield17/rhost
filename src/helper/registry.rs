//! Shared native namespace discovery and explicit retention management.
use crate::{
    domain::{Phase, Recovery, Submission},
    remote::Error,
    transport::Client,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Runs,
    Changesets,
    Queries,
}
impl Kind {
    fn text(self) -> &'static str {
        match self {
            Self::Runs => "runs",
            Self::Changesets => "changesets",
            Self::Queries => "queries",
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Page {
    generation: String,
    #[serde(default, deserialize_with = "present")]
    limit: Option<u16>,
    #[serde(default, deserialize_with = "present")]
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    generation: String,
    kind: Kind,
    #[serde(default, deserialize_with = "present")]
    limit: Option<u16>,
    #[serde(default, deserialize_with = "present")]
    cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Namespace {
    generation: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prune {
    generation: String,
    kind: Kind,
    ids: Vec<String>,
    #[serde(default = "yes")]
    dry_run: bool,
}
fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}
fn yes() -> bool {
    true
}
fn invalid() -> Error {
    Error::new("CONFIG_INVALID", "invalid namespace management request")
        .with_recovery(Recovery::invalid())
}
fn bad() -> Error {
    Error::new("PROTOCOL_ERROR", "invalid namespace management answer")
}
fn parse<T: for<'a> Deserialize<'a>>(raw: &str) -> Result<T, Error> {
    serde_json::from_str(raw).map_err(|_| invalid())
}
pub fn generation(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub fn key(s: &str) -> Option<u64> {
    let (expiry, random) = s.split_once('-')?;
    let n = expiry.parse().ok()?;
    (n > 0
        && expiry.len() <= 12
        && expiry.bytes().all(|c| c.is_ascii_digit())
        && !expiry.starts_with('0')
        && random.len() == 64
        && random
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
    .then_some(n)
}
fn cursor(s: &str, g: &str, k: Kind) -> bool {
    s.strip_prefix(&format!("{g}:{}:", k.text()))
        .is_some_and(|id| key(id).is_some())
}
pub fn validate(operation: &str, raw: &str) -> Result<Value, Error> {
    let mut q = match operation {
        "run.list" | "fs.changeset.list" => {
            let p: Page = parse(raw)?;
            json!({"op":"state.list","generation":p.generation,"kind":if operation == "run.list" {Kind::Runs} else {Kind::Changesets},"limit":p.limit.unwrap_or(64),"cursor":p.cursor})
        }
        "state.list" => {
            let p: Listing = parse(raw)?;
            json!({"op":operation,"generation":p.generation,"kind":p.kind,"limit":p.limit.unwrap_or(64),"cursor":p.cursor})
        }
        "state.usage" => {
            let p: Namespace = parse(raw)?;
            json!({"op":operation,"generation":p.generation})
        }
        "state.prune" => {
            let p: Prune = parse(raw)?;
            if p.ids.is_empty()
                || p.ids.len() > 64
                || p.ids.iter().any(|id| key(id).is_none())
                || p.ids
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != p.ids.len()
            {
                return Err(invalid());
            }
            json!({"op":operation,"generation":p.generation,"kind":p.kind,"ids":p.ids,"dry_run":p.dry_run})
        }
        _ => return Err(invalid()),
    };
    let g = q["generation"].as_str().ok_or_else(invalid)?;
    if !generation(g) {
        return Err(invalid());
    }
    if q["op"] == "state.list" {
        let k: Kind = serde_json::from_value(q["kind"].clone()).map_err(|_| invalid())?;
        if !q["limit"].as_u64().is_some_and(|n| (1..=256).contains(&n))
            || q["cursor"].as_str().is_some_and(|c| !cursor(c, g, k))
        {
            return Err(invalid());
        }
        if q["cursor"].is_null() {
            q.as_object_mut().ok_or_else(invalid)?.remove("cursor");
        }
    }
    Ok(q)
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Detail {
    code: String,
    message: String,
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    kind: Kind,
    expires_unix: u64,
    status: String,
    owner_active: bool,
    logical_bytes: u64,
    #[serde(deserialize_with = "required")]
    error: Option<Detail>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PageResult {
    generation: String,
    items: Vec<Entry>,
    #[serde(deserialize_with = "required")]
    next_cursor: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    records: u64,
    reserved_bytes: u64,
    collections: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Usage {
    generation: String,
    logical_bytes: u64,
    reserved_bytes: u64,
    records: u64,
    collections: u64,
    complete: bool,
    limits: Limits,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    eligible: bool,
    reason: String,
    pruned: bool,
    #[serde(deserialize_with = "required")]
    error: Option<Detail>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pruned {
    generation: String,
    kind: Kind,
    dry_run: bool,
    items: Vec<Receipt>,
    failed: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Refused {
    error: String,
    message: String,
    mutation: String,
}
fn refusal(r: Refused, observation: bool) -> Error {
    let code = match r.error.as_str() {
        "CONFIG_INVALID" => "CONFIG_INVALID",
        "NAMESPACE_MISMATCH" => "NAMESPACE_MISMATCH",
        "STATE_UNAVAILABLE" => "STATE_UNAVAILABLE",
        "STATE_CAPACITY_EXCEEDED" => "STATE_CAPACITY_EXCEEDED",
        "CONTEXT_CHANGED" => "CONTEXT_CHANGED",
        "CONTEXT_UNAVAILABLE" => "CONTEXT_UNAVAILABLE",
        "INTERNAL" => "INTERNAL",
        _ => return bad(),
    };
    if crate::domain::MutationEvidence::parse(&r.mutation).is_none() {
        return bad();
    }
    Error::new(code, r.message).with_recovery(if observation {
        Recovery::observe(Phase::Operation, Submission::Confirmed)
    } else {
        Recovery::inspect(Phase::Operation, Submission::Confirmed)
    })
}
pub fn execute(
    client: &Client,
    host: &str,
    operation: &str,
    raw: &str,
    cancelled: &dyn Fn() -> bool,
    guard: Option<&crate::domain::BootId>,
) -> Result<Value, Error> {
    let q = validate(operation, raw)?;
    let observation = q["op"] != "state.prune" || q["dry_run"] == true;
    let bytes = serde_json::to_vec(&q).map_err(|_| invalid())?;
    let reply = super::Helper::new(client, host)
        .with_cancellation(cancelled)
        .with_boot_guard(guard)
        .file_call(&bytes, Duration::from_secs(60), 1024 * 1024)
        .map_err(|mut e| {
            e.mutation = None;
            if observation && e.recovery.submission() != "not_submitted" {
                e.recovery = Recovery::observe(Phase::Observation, Submission::MayHaveSubmitted);
            }
            e
        })?;
    if let Ok(r) = serde_json::from_slice::<Refused>(&reply) {
        return Err(refusal(r, observation));
    }
    let value = match q["op"].as_str() {
        Some("state.list") => {
            let p: PageResult = serde_json::from_slice(&reply).map_err(|_| bad())?;
            let kind: Kind = serde_json::from_value(q["kind"].clone()).map_err(|_| bad())?;
            let after = q["cursor"]
                .as_str()
                .and_then(|s| s.rsplit(':').next())
                .unwrap_or("");
            if p.generation != q["generation"]
                || p.items.len() > q["limit"].as_u64().unwrap_or(0) as usize
                || p.items.iter().any(|e| {
                    e.kind != kind
                        || key(&e.id) != Some(e.expires_unix)
                        || e.status.is_empty()
                        || e.error
                            .as_ref()
                            .is_some_and(|e| e.code != "STATE_UNAVAILABLE")
                })
                || p.items.windows(2).any(|w| w[0].id >= w[1].id)
                || p.items.first().is_some_and(|e| e.id.as_str() <= after)
            {
                return Err(bad());
            }
            if let Some(c) = &p.next_cursor {
                if !cursor(c, &p.generation, kind)
                    || p.items
                        .last()
                        .is_none_or(|e| !c.ends_with(&format!(":{}", e.id)))
                {
                    return Err(bad());
                }
            }
            serde_json::to_value(p)
        }
        Some("state.usage") => {
            let p: Usage = serde_json::from_slice(&reply).map_err(|_| bad())?;
            if p.generation != q["generation"]
                || p.reserved_bytes < p.logical_bytes
                || p.collections > p.records
                || (
                    p.limits.records,
                    p.limits.reserved_bytes,
                    p.limits.collections,
                ) != (16384, 8589934592, 32)
            {
                return Err(bad());
            }
            serde_json::to_value(p)
        }
        Some("state.prune") => {
            let p: Pruned = serde_json::from_slice(&reply).map_err(|_| bad())?;
            let kind: Kind = serde_json::from_value(q["kind"].clone()).map_err(|_| bad())?;
            let ids = q["ids"].as_array().ok_or_else(bad)?;
            if p.generation != q["generation"]
                || p.kind != kind
                || p.dry_run != q["dry_run"]
                || p.items.len() != ids.len()
                || p.failed != p.items.iter().filter(|e| e.error.is_some()).count() as u64
                || p.items.iter().zip(ids).any(|(r, id)| {
                    r.id != *id || (r.pruned && (!r.eligible || p.dry_run || r.error.is_some()))
                })
            {
                return Err(bad());
            }
            serde_json::to_value(p)
        }
        _ => return Err(bad()),
    };
    value.map_err(|_| bad())
}
