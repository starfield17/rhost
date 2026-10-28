//! Validate private evidence before exposing a public observation.
use crate::helper::Reader;
use crate::remote::Error;
use serde_json::{Value, json};
fn protocol(message: &str) -> Error {
    crate::helper::codec::protocol(message)
}
pub fn bind(
    kind: u32,
    data: &Value,
    generation: Option<&str>,
    key: Option<&str>,
) -> Result<(), Error> {
    if matches!(kind, 2 | 3)
        && (data["run"]["generation"].as_str() != generation
            || data["run"]["run_id"].as_str() != key)
    {
        return Err(protocol("foreign Run observation"));
    }
    if kind == 5 && data["run_id"].as_str() != key {
        return Err(protocol("foreign signal receipt"));
    }
    if kind == 4
        && data.as_array().is_none_or(|items| {
            items
                .iter()
                .any(|item| item["generation"].as_str() != generation)
        })
    {
        return Err(protocol("foreign Run list"));
    }
    Ok(())
}
fn record(r: &mut Reader<'_>) -> Result<Value, Error> {
    let generation = r.text()?;
    let run_id = r.text()?;
    super::request::generation(&generation)?;
    super::request::key(&run_id)?;
    let status = match r.byte()? {
        1 => "accepted",
        2 => "starting",
        3 => "running",
        4 => "exited",
        5 => "lost",
        _ => return Err(protocol("invalid Run state")),
    };
    let mut value = json!({"generation":generation,"run_id":run_id,"status":status});
    if status == "exited" {
        let kind = r.byte()?;
        let n = r.int()?;
        value["completion"] = match (kind, n) {
            (1, 0..=255) => json!({"exit_code":n}),
            (2, 1..) => json!({"signal":n}),
            _ => return Err(protocol("invalid completion")),
        };
    }
    Ok(value)
}
fn stream(r: &mut Reader<'_>) -> Result<Value, Error> {
    let from = r.long()?;
    let next = r.long()?;
    let observed = r.long()?;
    let bytes = r.bytes()?;
    let open = r.boolean()?;
    let incomplete = r.boolean()?;
    if next < from || next - from != bytes.len() as u64 || next > observed {
        return Err(protocol("inconsistent output cursor"));
    }
    Ok(
        json!({"from":from,"next":next,"observed_bytes":observed,"content_base64":crate::base64::encode(bytes),
        "content":String::from_utf8_lossy(bytes),"open":open,"incomplete":incomplete}),
    )
}
pub fn decode(kind: u32, bytes: &[u8]) -> Result<Value, Error> {
    let mut r = Reader::new(bytes);
    let data = match kind {
        7 | 8 => {
            let generation = r.text()?;
            super::request::generation(&generation)?;
            let now = r.long()?;
            let boot = r.text()?;
            if !boot.is_empty() {
                crate::domain::BootId::new(boot.clone())
                    .map_err(|_| protocol("invalid namespace boot identity"))?;
            }
            json!({"schema_generations":[3],"generation":generation,"remote_time_unix":now,"context":{"boot_id":if boot.is_empty(){None}else{Some(boot)}}})
        }
        2 | 3 => {
            json!({"run":record(&mut r)?,"stdout":stream(&mut r)?,"stderr":stream(&mut r)?,"stale":false})
        }
        4 => {
            let count = r.int()?;
            if count > 4096 {
                return Err(protocol("excessive Run list"));
            }
            let mut items = Vec::new();
            for _ in 0..count {
                items.push(record(&mut r)?);
            }
            json!(items)
        }
        5 => {
            let id = r.text()?;
            super::request::key(&id)?;
            if !r.boolean()? {
                return Err(protocol("signal delivery not confirmed"));
            }
            json!({"run_id":id,"delivered":true})
        }
        6 => {
            let count = r.int()?;
            if count > 4096 {
                return Err(protocol("excessive prune list"));
            }
            let mut ids = Vec::new();
            for _ in 0..count {
                let id = r.text()?;
                super::request::key(&id)?;
                ids.push(id);
            }
            json!({"removed":ids})
        }
        _ => return Err(protocol("unknown operation")),
    };
    r.done()?;
    Ok(data)
}
