//! Structured, bounded observations; revisions are not workspace snapshots.
use super::{
    RequestOutcome,
    backend::{Answer, ReadResult},
};
use crate::{remote::Error, transport::Client};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
fn cap() -> usize {
    262144
}
fn one() -> u64 {
    1
}
fn lines() -> u64 {
    200
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
    serde_json::from_str(raw).map_err(|e| {
        Error::new("CONFIG_INVALID", e.to_string())
            .with_recovery(crate::domain::Recovery::invalid())
    })
}
fn invalid() -> Error {
    Error::new("CONFIG_INVALID", "invalid structured inspection bounds")
        .with_recovery(crate::domain::Recovery::invalid())
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
pub(super) fn call<T: for<'a> Deserialize<'a>>(
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
    raw: &str,
    cancelled: &dyn Fn() -> bool,
    guard: Option<&crate::domain::BootId>,
) -> RequestOutcome {
    let result = (|| {
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
    })();
    match result {
        Ok(data) => RequestOutcome::done(data),
        Err(mut error) => {
            error.mutation = None;
            if error.recovery.submission() != "not_submitted" {
                error.recovery = crate::domain::Recovery::observe(
                    crate::domain::Phase::Observation,
                    if error.recovery.submission() == "confirmed" {
                        crate::domain::Submission::Confirmed
                    } else {
                        crate::domain::Submission::MayHaveSubmitted
                    },
                );
            }
            RequestOutcome::failed(error)
        }
    }
}
