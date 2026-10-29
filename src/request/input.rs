//! Target-aware outer documents. Capability parameters remain capability-owned.
use crate::remote::Error;
use serde::Deserialize;
use serde_json::{Value, value::RawValue};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    pub schema_version: u8,
    pub operation: String,
    #[serde(default, deserialize_with = "present")]
    pub host: Option<String>,
    pub params: Box<RawValue>,
    #[serde(default, deserialize_with = "present")]
    pub if_boot_id: Option<String>,
}
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    #[serde(default, deserialize_with = "present")]
    host: Option<String>,
    params: Value,
    #[serde(default, deserialize_with = "present")]
    if_boot_id: Option<String>,
}
pub(crate) fn raw(raw: &str) -> Result<Call, Error> {
    if !raw.trim_start().starts_with('{') {
        return Err(invalid("request must be a JSON object"));
    }
    if raw.len() > 16 * 1024 * 1024 {
        return Err(invalid("JSON request exceeds 16 MiB"));
    }
    let call: Call = serde_json::from_str(raw).map_err(|e| invalid(e.to_string()))?;
    validate(call)
}
pub(crate) fn arguments(operation: &str, arguments: Value) -> Result<Call, Error> {
    if arguments.to_string().len() > 16 * 1024 * 1024 {
        return Err(invalid("JSON request exceeds 16 MiB"));
    }
    let args: Arguments = serde_json::from_value(arguments).map_err(|e| invalid(e.to_string()))?;
    validate(Call {
        schema_version: 4,
        operation: operation.into(),
        host: args.host,
        params: serde_json::value::to_raw_value(&args.params)
            .map_err(|e| invalid(e.to_string()))?,
        if_boot_id: args.if_boot_id,
    })
}
fn validate(call: Call) -> Result<Call, Error> {
    if !call.params.get().trim_start().starts_with('{') {
        return Err(invalid("params must be a JSON object"));
    }
    if call.schema_version != 4 {
        return Err(invalid("request requires schema v4"));
    }
    if matches!(call.operation.as_str(), "version" | "hosts") {
        if call.host.is_some() || call.if_boot_id.is_some() {
            return Err(invalid("local discovery has no host or boot guard"));
        }
    } else {
        if call
            .host
            .as_deref()
            .is_none_or(|host| host.is_empty() || host.contains('\0'))
        {
            return Err(invalid("remote operation requires a host"));
        }
        if super::core::recognizes(&call.operation) && call.if_boot_id.is_some() {
            return Err(invalid("this operation does not support a boot guard"));
        }
    }
    Ok(call)
}
fn invalid(message: impl Into<String>) -> Error {
    Error::new("CONFIG_INVALID", message).with_recovery(crate::domain::Recovery::invalid())
}
