//! Delivery-neutral composition for CLI JSON and MCP; capability types stay owned.
use crate::remote::Error;
use crate::transport::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Read;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u8,
    operation: String,
    host: String,
    params: Box<serde_json::value::RawValue>,
    #[serde(default, deserialize_with = "nonnull_boot")]
    if_boot_id: Option<String>,
}
fn nonnull_boot<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}
pub fn execute(client: &Client, raw: &str, cancelled: &dyn Fn() -> bool) -> Value {
    let prepared = (|| {
        if raw.len() > 16 * 1024 * 1024 {
            return Err(Error::new("CONFIG_INVALID", "JSON request exceeds 16 MiB"));
        }
        let q: Document =
            serde_json::from_str(raw).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        if q.schema_version != 3 || q.host.is_empty() || q.host.contains('\0') {
            return Err(Error::new(
                "CONFIG_INVALID",
                "request requires schema v3 and a host",
            ));
        }
        Ok(q)
    })();
    let q = match prepared {
        Ok(q) => q,
        Err(e) => return failure("request", None, e),
    };
    let guard = match q.if_boot_id.map(crate::domain::BootId::new).transpose() {
        Ok(value) => value,
        Err(e) => return invalid(e.to_string()),
    };
    let result = if matches!(
        q.operation.as_str(),
        "fs.read" | "fs.write" | "fs.patch" | "fs.search" | "fs.read_many"
    ) || matches!(
        q.operation.as_str(),
        "fs.changeset.apply" | "fs.changeset.query" | "fs.changeset.recover" | "fs.changeset.prune"
    ) {
        let outcome = crate::files::execute_params(
            client,
            &q.host,
            &q.operation,
            q.params.get(),
            cancelled,
            guard.as_ref(),
        );
        return outcome.envelope(&q.operation, &q.host);
    } else if matches!(
        q.operation.as_str(),
        "metadata"
            | "helper.prepare"
            | "run.start"
            | "run.poll"
            | "run.list"
            | "run.prune"
            | "run.signal"
    ) {
        crate::runs::execute_params(
            client,
            &q.host,
            &q.operation,
            q.params.get(),
            cancelled,
            guard.as_ref(),
        )
    } else {
        return failure(
            "request",
            Some(&q.host),
            Error::new("USAGE_ERROR", "unknown structured operation"),
        );
    };
    match result {
        Ok(data) => {
            json!({"schema_version":3,"operation":q.operation,"ok":true,"host":q.host,"data":data,"error":null})
        }
        Err(e) => failure(&q.operation, Some(&q.host), e),
    }
}
pub fn failure(operation: &str, host: Option<&str>, error: Error) -> Value {
    let mut detail = json!({"code":error.code,"message":error.message,"retryable":error.retryable});
    if let Some(mutation) = error.mutation {
        detail["mutation"] = mutation.as_str().into();
    } else if matches!(operation, "fs.write" | "fs.patch") {
        detail["mutation"] = "not_applied".into();
    }
    json!({"schema_version":3,"operation":operation,"ok":false,"host":host,"data":null,"error":detail})
}
pub fn invalid(message: impl Into<String>) -> Value {
    failure("request", None, Error::new("CONFIG_INVALID", message))
}
pub fn observation_cancelled(operation: &str, host: &str) -> Value {
    failure(
        operation,
        Some(host),
        Error::new(
            "REMOTE_COMMAND_CANCELLED",
            "local observation cancelled before submission",
        ),
    )
}
pub fn run(
    sink: &mut crate::cli::Sink,
    client: &Client,
    invocation: crate::runs::Invocation,
    interrupt: &crate::signals::Interrupt,
) -> u8 {
    if invocation.help && !invocation.json {
        sink.text(&invocation.help_text);
        return 0;
    }
    let input = match invocation.input {
        Ok(crate::runs::command::Input::Document(raw)) => Ok(raw),
        Ok(crate::runs::command::Input::File(path)) => {
            let reader: Result<Box<dyn Read>, std::io::Error> = if path == "-" {
                Ok(Box::new(std::io::stdin()))
            } else {
                std::fs::File::open(path).map(|f| Box::new(f) as Box<dyn Read>)
            };
            reader.map_err(|e| e.to_string()).and_then(|r| {
                let mut bytes = Vec::new();
                r.take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 16 * 1024 * 1024 {
                    return Err("request exceeds 16 MiB".into());
                }
                String::from_utf8(bytes).map_err(|e| e.to_string())
            })
        }
        Err(e) => Err(e),
    };
    let value = match input {
        Ok(raw) => execute(client, &raw, &|| interrupt.requested()),
        Err(e) => failure("request", None, Error::new("USAGE_ERROR", e)),
    };
    let status = if value["ok"] == true { 0 } else { 255 };
    let rendered = if invocation.json {
        serde_json::to_string(&value)
    } else {
        serde_json::to_string_pretty(&value)
    };
    match rendered {
        Ok(text) => sink.line(&text),
        Err(e) => {
            crate::cli::warn(&format!("rhost: OUTPUT_WRITE_FAILED: {e}"));
            return 255;
        }
    }
    status
}
