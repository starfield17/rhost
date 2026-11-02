//! Structured file parameters are parsed once, before helper preparation.
use super::backend::{Edit, WriteResult};
use crate::remote::Error;
use crate::transport::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    path: String,
    #[serde(default, deserialize_with = "optional_number")]
    offset_bytes: Option<u64>,
    #[serde(default, deserialize_with = "optional_hash")]
    if_hash: Option<String>,
    #[serde(default, deserialize_with = "optional_number")]
    timeout_ms: Option<u64>,
    #[serde(default = "one")]
    start: u64,
    #[serde(default = "lines")]
    lines: u64,
    #[serde(default = "cap")]
    max_bytes: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    path: String,
    content: String,
    #[serde(default, deserialize_with = "supplied_hash")]
    if_hash: String,
    #[serde(default)]
    parents: bool,
    #[serde(default, deserialize_with = "nonnull_mode")]
    mode: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    path: String,
    if_hash: String,
    edits: Vec<Edit>,
}
fn optional_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(d).map(Some)
}
fn optional_hash<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
fn nonnull_mode<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
fn supplied_hash<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let text = String::deserialize(d)?;
    crate::domain::Sha256::new(text.clone()).map_err(serde::de::Error::custom)?;
    Ok(text)
}
fn hash(s: &str) -> Result<(), Error> {
    crate::domain::Sha256::new(s.to_string())
        .map(|_| ())
        .map_err(|e| {
            Error::new("CONFIG_INVALID", e.to_string())
                .with_recovery(crate::domain::Recovery::invalid())
        })
}
fn one() -> u64 {
    1
}
fn lines() -> u64 {
    200
}
fn cap() -> usize {
    8192
}
fn parse<T: for<'a> Deserialize<'a>>(s: &str) -> Result<T, Error> {
    serde_json::from_str(s).map_err(|e| {
        Error::new("CONFIG_INVALID", e.to_string())
            .with_recovery(crate::domain::Recovery::invalid())
    })
}
fn path(s: &str) -> Result<(), Error> {
    super::backend::validate_remote_path(s).map_err(|e| {
        Error::new("CONFIG_INVALID", e.to_string())
            .with_recovery(crate::domain::Recovery::invalid())
    })
}
fn execute_simple(
    client: &Client,
    host: &str,
    operation: &str,
    raw: &str,
    cancelled: &dyn Fn() -> bool,
    boot_guard: Option<&crate::domain::BootId>,
) -> Result<Value, Error> {
    let result = (|| {
        let (payload, budget) = (|| -> Result<_, Error> {
        let (payload, budget) = match operation {
            "fs.read" => {
                let p: Read = parse(raw)?;
                path(&p.path)?;
                if p.start == 0 || p.lines == 0 || p.max_bytes == 0 || p.max_bytes > 8 * 1024 * 1024
                {
                    return Err(Error::new("CONFIG_INVALID", "invalid read bounds"));
                }
                if let Some(ref h)=p.if_hash {hash(h)?;}
                let mut v=if let Some(offset)=p.offset_bytes {
                    if p.start!=1 || p.lines!=200 || p.max_bytes<4{return Err(Error::new("CONFIG_INVALID","byte and line ranges are exclusive"));}
                    json!({"op":"read","path":p.path,"offset_bytes":offset,"max_bytes":p.max_bytes})
                }else {json!({"op":"read","path":p.path,"start":p.start,"lines":p.lines,"max_bytes":p.max_bytes,"with_coverage":true})};
                if let Some(h)=p.if_hash {if p.offset_bytes.is_none(){return Err(Error::new("CONFIG_INVALID","hash guard requires byte offset"));}v["if_hash"]=json!(h);}
                let _timeout=p.timeout_ms;
                (v,p.max_bytes)
            }
            "fs.write" => {
                let p: Write = parse(raw)?;
                path(&p.path)?;
                if !p.if_hash.is_empty() {
                    hash(&p.if_hash)?;
                }
                if p.content.len() > 8 * 1024 * 1024 {
                    return Err(Error::new("FILE_TOO_LARGE", "input exceeds 8 MiB"));
                }
                if p.mode
                    .as_deref()
                    .is_some_and(|m| !super::ops::is_octal_mode(m))
                {
                    return Err(Error::new("CONFIG_INVALID", "invalid octal mode"));
                }
                let mut value = json!({"op":"write","path":p.path,"content":crate::base64::encode(p.content.as_bytes()),"if_hash":p.if_hash,"parents":p.parents,"max_bytes":4096});
                if let Some(mode) = p.mode {
                    value["file_mode"] = mode.into();
                }
                (value, 4096)
            }
            "fs.patch" => {
                let p: Patch = parse(raw)?;
                path(&p.path)?;
                if p.if_hash.is_empty() || p.edits.is_empty() {
                    return Err(Error::new(
                        "CONFIG_INVALID",
                        "patch requires hash and edits",
                    ));
                }
                hash(&p.if_hash)?;
                super::input::validate_edits(&p.edits)?;
                (
                    json!({"op":"patch","path":p.path,"if_hash":p.if_hash,"edits":p.edits,"max_bytes":4096}),
                    4096,
                )
            }
            _ => {
                return Err(Error::new(
                    "USAGE_ERROR",
                    "unknown structured file operation",
                ));
            }
        };
        let payload =
            serde_json::to_vec(&payload).map_err(|e| Error::new("INTERNAL", e.to_string()))?;
        if payload.len() > 12 * 1024 * 1024 {
            return Err(Error::new(
                "FILE_TOO_LARGE",
                "encoded file request exceeds 12 MiB",
            ));
        }
        Ok((payload, budget))
        })().map_err(|e| e.with_recovery(crate::domain::Recovery::invalid()))?;
        let timeout = if operation == "fs.read" {
            parse::<Read>(raw)?
                .timeout_ms
                .filter(|n| *n != 0)
                .unwrap_or(60000)
        } else {
            30000
        };
        let bytes = crate::helper::Helper::new(client, host)
            .with_cancellation(cancelled)
            .with_boot_guard(boot_guard)
            .file_call(
                &payload,
                Duration::from_millis(timeout),
                super::backend::output_budget(budget),
            )?;
        if operation == "fs.read" {
            let answer: Value = serde_json::from_slice(&bytes).map_err(|_| bad_answer())?;
            if answer.get("error").is_some() {
                return Err(refusal(
                    serde_json::from_value(answer).map_err(|_| bad_answer())?,
                ));
            }
            let requested: Read = parse(raw)?;
            if requested
                .offset_bytes
                .is_some_and(|n| answer["coverage"]["from"].as_u64() != Some(n))
            {
                return Err(bad_answer());
            }
            super::page::validate(
                answer,
                &requested.path,
                budget,
                requested
                    .offset_bytes
                    .is_none()
                    .then_some((requested.start, requested.lines)),
            )
        } else {
            let answer: super::backend::Answer<WriteResult> =
                serde_json::from_slice(&bytes).map_err(|_| bad_answer())?;
            match answer {
                super::backend::Answer::Done(p) => Ok(
                    json!({"path":p.path,"sha256":p.sha256,"bytes":p.bytes,"mutation":"applied"}),
                ),
                super::backend::Answer::Refused(e) => Err(refusal(e)),
            }
        }
    })();
    result.map_err(|mut error: Error| {
        if operation == "fs.read" {
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
        }
        error
    })
}
fn bad_answer() -> Error {
    Error::new("PROTOCOL_ERROR", "invalid native file answer")
        .with_mutation(crate::domain::MutationEvidence::Unknown)
}
pub(super) fn refusal(e: super::backend::HelperRefusal) -> Error {
    let Some(code) = super::backend::known_refusal_code(&e.error) else {
        return bad_answer();
    };
    let Some(mutation) = e
        .mutation
        .as_deref()
        .and_then(crate::domain::MutationEvidence::parse)
    else {
        return bad_answer();
    };
    Error::new(code, e.message())
        .with_mutation(mutation)
        .with_recovery(crate::domain::Recovery::inspect(
            crate::domain::Phase::Operation,
            crate::domain::Submission::Confirmed,
        ))
}

/// A partial journal survives a failed operation and must reach every delivery.
pub struct RequestOutcome {
    pub data: Value,
    pub error: Option<Error>,
}
impl RequestOutcome {
    pub fn envelope(self, operation: &str, host: &str) -> Value {
        let error = self.error.map(|mut e| {
            if (matches!(operation, "fs.write" | "fs.patch")
                || operation.starts_with("fs.changeset."))
                && e.mutation.is_none()
            {
                e.mutation = Some(crate::domain::MutationEvidence::NotApplied);
            }
            let mut value = json!({"code":e.code,"message":e.message,"retryable":e.retryable,"recovery":crate::wire::recovery(e.recovery)});
            if let Some(m) = e.mutation {
                value["mutation"] = m.as_str().into();
            }
            value
        });
        json!({"schema_version":5,"operation":operation,"host":host,"ok":error.is_none(),"data":self.data,"error":error})
    }
    pub fn done(data: Value) -> Self {
        Self { data, error: None }
    }
    pub fn failed(error: Error) -> Self {
        Self {
            data: Value::Null,
            error: Some(error),
        }
    }
}
pub fn execute_params(
    client: &Client,
    host: &str,
    operation: &str,
    raw: &str,
    cancelled: &dyn Fn() -> bool,
    guard: Option<&crate::domain::BootId>,
) -> RequestOutcome {
    let mut outcome = if operation == "fs.changeset.list" {
        match crate::helper::registry::execute(client, host, operation, raw, cancelled, guard) {
            Ok(data) => RequestOutcome::done(data),
            Err(e) => RequestOutcome::failed(e),
        }
    } else if operation.starts_with("fs.changeset.") {
        super::changes::execute(client, host, operation, raw, cancelled, guard)
    } else if matches!(operation, "fs.search" | "fs.find") {
        super::collection::execute(client, host, operation, raw, cancelled, guard)
    } else if operation == "fs.read_many" {
        super::inspect::execute(client, host, raw, cancelled, guard)
    } else {
        match execute_simple(client, host, operation, raw, cancelled, guard) {
            Ok(data) => RequestOutcome::done(data),
            Err(e) => RequestOutcome::failed(e),
        }
    };
    if (matches!(operation, "fs.write" | "fs.patch") || operation.starts_with("fs.changeset."))
        && let Some(e) = outcome.error.as_mut()
        && e.mutation.is_none()
    {
        e.mutation = Some(crate::domain::MutationEvidence::NotApplied);
    }
    outcome
}
