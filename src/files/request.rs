//! Structured file parameters are parsed once, before helper preparation.
use super::backend::{Edit, ReadResult, WriteResult};
use crate::remote::Error;
use crate::transport::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    path: String,
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
        .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))
}
fn one() -> u64 {
    1
}
fn lines() -> u64 {
    200
}
fn cap() -> usize {
    256 * 1024
}
fn parse<T: for<'a> Deserialize<'a>>(s: &str) -> Result<T, Error> {
    serde_json::from_str(s).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))
}
fn path(s: &str) -> Result<(), Error> {
    super::backend::validate_remote_path(s).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))
}
pub fn execute_params(
    client: &Client,
    host: &str,
    operation: &str,
    raw: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Value, Error> {
    let result = (|| {
        let (payload, budget) = match operation {
            "fs.read" => {
                let p: Read = parse(raw)?;
                path(&p.path)?;
                if p.start == 0 || p.lines == 0 || p.max_bytes == 0 || p.max_bytes > 8 * 1024 * 1024
                {
                    return Err(Error::new("CONFIG_INVALID", "invalid read bounds"));
                }
                (
                    json!({"op":"read","path":p.path,"start":p.start,"lines":p.lines,"max_bytes":p.max_bytes}),
                    p.max_bytes,
                )
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
        let bytes = crate::helper::Helper::new(client, host)
            .with_cancellation(cancelled)
            .file_call(
                &payload,
                Duration::from_secs(30),
                super::backend::output_budget(budget),
            )?;
        if operation == "fs.read" {
            let answer: super::backend::Answer<ReadResult> =
                serde_json::from_slice(&bytes).map_err(|_| bad_answer())?;
            match answer {
                super::backend::Answer::Done(p) => Ok(
                    json!({"path":p.path,"sha256":p.sha256,"content":p.content,"start":p.start,"lines":p.lines,"total_lines":p.total_lines,"truncated":p.truncated}),
                ),
                super::backend::Answer::Refused(e) => Err(refusal(e)),
            }
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
        if !(operation != "fs.read") {
            error.mutation = None;
        }
        error
    })
}
fn bad_answer() -> Error {
    Error::new("PROTOCOL_ERROR", "invalid native file answer")
        .with_mutation(crate::domain::MutationEvidence::Unknown)
}
fn refusal(e: super::backend::HelperRefusal) -> Error {
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
    Error::new(code, e.message()).with_mutation(mutation)
}
