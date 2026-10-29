//! Closed JSON parameters become the same operation used by the CLI adapter.
use super::application::{
    CREATE_TIMEOUT, EXEC_TIMEOUT, LIST_TIMEOUT, READ_TIMEOUT, RECOVER_TIMEOUT, Request,
};
use crate::remote::Error;
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Duration;

fn parse<T: DeserializeOwned>(raw: &str) -> Result<T, Error> {
    serde_json::from_str(raw).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))
}
pub(crate) fn timeout(ms: u64, seconds: u64) -> Result<Duration, Error> {
    if ms > 9_223_372_036_854 {
        return Err(Error::new(
            "CONFIG_INVALID",
            "timeout exceeds duration range",
        ));
    }
    Ok(if ms == 0 {
        Duration::from_secs(seconds)
    } else {
        Duration::from_millis(ms)
    })
}
fn optional<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(d).map(Some)
}
fn boolean<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    bool::deserialize(d).map(Some)
}
fn text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    #[serde(default)]
    name: String,
    #[serde(default)]
    cwd: String,
    #[serde(default = "shell")]
    shell: String,
    #[serde(default)]
    timeout_ms: u64,
}
fn shell() -> String {
    "bash".into()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    #[serde(default)]
    timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exec {
    session: String,
    command: String,
    #[serde(default)]
    timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    session: String,
    #[serde(default, deserialize_with = "text")]
    data: Option<String>,
    #[serde(default, deserialize_with = "text")]
    key: Option<String>,
    #[serde(default, deserialize_with = "boolean")]
    enter: Option<bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    session: String,
    #[serde(default, deserialize_with = "optional")]
    since: Option<u64>,
    #[serde(default, deserialize_with = "optional")]
    tail: Option<u64>,
    #[serde(default = "page")]
    max_bytes: usize,
    #[serde(default)]
    timeout_ms: u64,
}
fn page() -> usize {
    super::MAX_READ_BYTES
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recover {
    session: String,
    #[serde(default)]
    timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Close {
    session: String,
}

pub(crate) fn prepare(operation: &str, raw: &str) -> Result<Request, Error> {
    let request = match operation {
        "session.create" => {
            let p: Create = parse(raw)?;
            Request::Create {
                name: p.name,
                cwd: p.cwd,
                shell: p.shell,
                timeout: timeout(p.timeout_ms, CREATE_TIMEOUT.as_secs())?,
            }
        }
        "session.list" => {
            let p: List = parse(raw)?;
            Request::List {
                timeout: timeout(p.timeout_ms, LIST_TIMEOUT.as_secs())?,
            }
        }
        "session.exec" => {
            let p: Exec = parse(raw)?;
            Request::Exec {
                session: p.session,
                command: p.command,
                timeout: timeout(p.timeout_ms, EXEC_TIMEOUT.as_secs())?,
            }
        }
        "session.send" => {
            let p: Send = parse(raw)?;
            if p.data.is_some() == p.key.is_some() || p.key.is_some() && p.enter.is_some() {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    "provide data with optional enter, or one key",
                ));
            }
            Request::Send {
                session: p.session,
                data: p.data,
                key: p.key,
                enter: p.enter.unwrap_or(false),
            }
        }
        "session.read" => {
            let p: Read = parse(raw)?;
            let offset = match (p.since, p.tail) {
                (Some(_), Some(_)) => {
                    return Err(Error::new(
                        "CONFIG_INVALID",
                        "since and tail are mutually exclusive",
                    ));
                }
                (_, Some(tail)) => super::ReadOffset::Tail(tail),
                (since, None) => super::ReadOffset::Since(since.unwrap_or(0)),
            };
            Request::Read {
                session: p.session,
                offset,
                max_bytes: p.max_bytes,
                timeout: timeout(p.timeout_ms, READ_TIMEOUT.as_secs())?,
            }
        }
        "session.recover" => {
            let p: Recover = parse(raw)?;
            Request::Recover {
                session: p.session,
                timeout: timeout(p.timeout_ms, RECOVER_TIMEOUT.as_secs())?,
            }
        }
        "session.close" => {
            let p: Close = parse(raw)?;
            Request::Close { session: p.session }
        }
        _ => return Err(Error::new("USAGE_ERROR", "unknown session operation")),
    };
    Ok(request)
}
