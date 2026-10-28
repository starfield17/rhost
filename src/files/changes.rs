//! Targeted file journal contracts, separate from project or Git ownership.
use super::{RequestOutcome, backend::Edit};
use crate::{remote::Error, transport::Client};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Apply {
    generation: String,
    key: String,
    expires_unix: u64,
    changes: Vec<Change>,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
enum Change {
    Write {
        path: String,
        content: String,
        #[serde(default, deserialize_with = "supplied_hash")]
        if_hash: String,
        #[serde(default, deserialize_with = "nonnull_string")]
        mode: Option<String>,
    },
    Patch {
        path: String,
        if_hash: String,
        edits: Vec<Edit>,
    },
}
fn nonnull_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}
fn supplied_hash<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let s = String::deserialize(d)?;
    crate::domain::Sha256::new(s.clone()).map_err(serde::de::Error::custom)?;
    Ok(s)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    generation: String,
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recover {
    generation: String,
    id: String,
    action: Action,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Action {
    Resume,
    Rollback,
}
pub(super) fn key_expiry(key: &str) -> Result<u64, Error> {
    let (time, token) = key.split_once('-').ok_or_else(invalid)?;
    if time.is_empty()
        || time.len() > 12
        || time.starts_with('0')
        || !time.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    crate::domain::Sha256::new(token.into()).map_err(|_| invalid())?;
    time.parse().map_err(|_| invalid())
}
fn invalid() -> Error {
    Error::new("CONFIG_INVALID", "invalid changeset identity or bounds")
}
fn generation(s: &str) -> Result<(), Error> {
    crate::domain::InvocationToken::new(s.into())
        .map(|_| ())
        .map_err(|_| invalid())
}
fn parse<T: for<'a> Deserialize<'a>>(raw: &str) -> Result<T, Error> {
    serde_json::from_str(raw).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))
}
fn prepare(operation: &str, raw: &str) -> Result<(Value, String, String), Error> {
    if operation == "fs.changeset.apply" {
        let p: Apply = parse(raw)?;
        generation(&p.generation)?;
        if key_expiry(&p.key)? != p.expires_unix || p.changes.is_empty() || p.changes.len() > 64 {
            return Err(invalid());
        }
        let mut changes = Vec::new();
        let mut bytes = 0usize;
        for change in p.changes {
            changes.push(match change {
            Change::Write{path,content,if_hash,mode}=>{
                super::backend::validate_remote_path(&path).map_err(|_| invalid())?;
                if !if_hash.is_empty(){crate::domain::Sha256::new(if_hash.clone()).map_err(|_|invalid())?;}
                bytes=bytes.saturating_add(content.len());
                if mode.as_deref().is_some_and(|s|!super::ops::is_octal_mode(s)){return Err(invalid());}
                let mut v=json!({"op":"write","path":path,"content":crate::base64::encode(content.as_bytes()),"if_hash":if_hash});
                if let Some(mode)=mode{v["file_mode"]=mode.into();}v
            }
            Change::Patch{path,if_hash,edits}=>{
                super::backend::validate_remote_path(&path).map_err(|_| invalid())?;crate::domain::Sha256::new(if_hash.clone()).map_err(|_|invalid())?;super::input::validate_edits(&edits)?;
                json!({"op":"patch","path":path,"if_hash":if_hash,"edits":edits})
            }
        });
        }
        if bytes > 8 * 1024 * 1024 {
            return Err(Error::new(
                "FILE_TOO_LARGE",
                "changeset replacements exceed 8 MiB",
            ));
        }
        Ok((
            json!({"op":"changeset.apply","generation":p.generation,"key":p.key,"expires_unix":p.expires_unix,"changes":changes}),
            p.generation,
            p.key,
        ))
    } else if operation == "fs.changeset.recover" {
        let p: Recover = parse(raw)?;
        generation(&p.generation)?;
        key_expiry(&p.id)?;
        Ok((
            json!({"op":"changeset.recover","generation":p.generation,"id":p.id,"action":p.action}),
            p.generation,
            p.id,
        ))
    } else {
        let p: Identity = parse(raw)?;
        generation(&p.generation)?;
        key_expiry(&p.id)?;
        Ok((
            json!({"op":operation.trim_start_matches("fs."),"generation":p.generation,"id":p.id}),
            p.generation,
            p.id,
        ))
    }
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Registered,
    Prepared,
    Applying,
    Applied,
    Partial,
    RollingBack,
    RolledBack,
    Refused,
    Unknown,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Pending,
    Installing,
    Installed,
    Restoring,
    Restored,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    state: Phase,
    #[serde(deserialize_with = "required")]
    before_hash: Option<String>,
    #[serde(deserialize_with = "required")]
    after_hash: Option<String>,
    ever_applied: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    code: String,
    message: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    generation: String,
    id: String,
    fingerprint: String,
    expires_unix: u64,
    status: Status,
    mutation: String,
    owner_active: bool,
    cleanup_pending: bool,
    files: Vec<Entry>,
    #[serde(deserialize_with = "required")]
    failure: Option<Failure>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Detail {
    code: String,
    message: String,
    mutation: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Answer {
    record: Record,
    #[serde(deserialize_with = "required")]
    error: Option<Detail>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pruned {
    generation: String,
    id: String,
    pruned: bool,
}
fn bad() -> Error {
    Error::new("PROTOCOL_ERROR", "invalid targeted changeset evidence")
        .with_mutation(crate::domain::MutationEvidence::Unknown)
}
fn decode(bytes: &[u8], operation: &str, expected_generation: &str, id: &str) -> RequestOutcome {
    let result = (|| {
        if let Ok(refused) = serde_json::from_slice::<super::backend::HelperRefusal>(bytes) {
            return Err(super::request::refusal(refused));
        }
        if operation == "fs.changeset.prune"
            && let Ok(p) = serde_json::from_slice::<Pruned>(bytes)
        {
            if p.generation != expected_generation || p.id != id || !p.pruned {
                return Err(bad());
            }
            return Ok(RequestOutcome::done(
                serde_json::to_value(p).map_err(|_| bad())?,
            ));
        }
        let answer: Answer = serde_json::from_slice(bytes).map_err(|_| bad())?;
        let r = &answer.record;
        if r.generation != expected_generation
            || r.id != id
            || r.expires_unix != key_expiry(id)?
            || r.files.is_empty()
            || r.files.len() > 64
        {
            return Err(bad());
        }
        crate::domain::Sha256::new(r.fingerprint.clone()).map_err(|_| bad())?;
        let mutation = crate::domain::MutationEvidence::parse(&r.mutation).ok_or_else(bad)?;
        for file in &r.files {
            if !file.path.starts_with('/') || file.path.contains('\0') {
                return Err(bad());
            }
            for h in [&file.before_hash, &file.after_hash].into_iter().flatten() {
                crate::domain::Sha256::new(h.clone()).map_err(|_| bad())?;
            }
        }
        let error = answer
            .error
            .map(|e| {
                let code = super::backend::known_refusal_code(&e.code).ok_or_else(bad)?;
                if e.mutation != r.mutation {
                    return Err(bad());
                }
                Ok(Error::new(code, e.message).with_mutation(mutation))
            })
            .transpose()?;
        Ok(RequestOutcome {
            data: serde_json::to_value(r).map_err(|_| bad())?,
            error,
        })
    })();
    match result {
        Ok(outcome) => outcome,
        Err(error) => RequestOutcome::failed(error),
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
        let (payload, generation, id) = prepare(operation, raw)?;
        let bytes = serde_json::to_vec(&payload).map_err(|_| invalid())?;
        if bytes.len() > 12 * 1024 * 1024 {
            return Err(Error::new(
                "FILE_TOO_LARGE",
                "encoded changeset exceeds 12 MiB",
            ));
        }
        let reply = crate::helper::Helper::new(client, host)
            .with_cancellation(cancelled)
            .with_boot_guard(guard)
            .file_call(&bytes, std::time::Duration::from_secs(60), 512 * 1024)?;
        Ok(decode(&reply, operation, &generation, &id))
    })();
    match result {
        Ok(outcome) => outcome,
        Err(error) => RequestOutcome::failed(error),
    }
}
