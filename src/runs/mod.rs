//! Recoverable Runs and structured requests; no durable local registry.
pub mod command;
mod dto;
#[cfg(test)]
mod dto_tests;
mod request;
#[cfg(test)]
mod request_tests;
use crate::cli::Sink;
use crate::helper::Helper;
use crate::remote::Error;
use crate::transport::Client;
pub use command::{FLAGS, Invocation, parse};
use serde_json::{Value, json};
use std::io::Read;
pub fn execute(client: &Client, raw: &str) -> Value {
    execute_cancellable(client, raw, &|| false)
}
pub fn execute_cancellable(client: &Client, raw: &str, cancelled: &dyn Fn() -> bool) -> Value {
    let prepared = match request::prepare(raw) {
        Ok(q) => q,
        Err(e) => return failure("request", None, e),
    };
    let host = prepared.host.clone();
    let operation = prepared.operation;
    let helper = Helper::new(client, &host).with_cancellation(cancelled);
    let result = (|| {
        let mut binding = crate::helper::Reader::new(&prepared.body.0);
        let expected_generation = if (2..=6).contains(&prepared.kind) {
            Some(binding.text()?)
        } else {
            None
        };
        let expected_key = if matches!(prepared.kind, 2 | 3 | 5) {
            Some(binding.text()?)
        } else {
            None
        };
        if prepared.kind != 8 {
            helper.ensure()?;
        } else {
            helper.probe()?;
        }
        let bytes = helper.call(prepared.kind, prepared.body, prepared.wait_ms)?;
        let data = dto::decode(prepared.kind, &bytes)?;
        dto::bind(
            prepared.kind,
            &data,
            expected_generation.as_deref(),
            expected_key.as_deref(),
        )?;
        Ok(data)
    })();
    match result {
        Ok(data) => {
            json!({"schema_version":3,"operation":operation,"ok":true,"host":host,"data":data,"error":null})
        }
        Err(error) => failure(operation, Some(&host), error),
    }
}
fn failure(operation: &str, host: Option<&str>, error: Error) -> Value {
    json!({"schema_version":3,"operation":operation,"ok":false,"host":host,"data":null,
        "error":{"code":error.code,"message":error.message,"retryable":error.retryable}})
}
pub fn run(
    sink: &mut Sink,
    client: &Client,
    invocation: Invocation,
    interrupt: &crate::signals::Interrupt,
) -> u8 {
    if invocation.help && !invocation.json {
        sink.text(&invocation.help_text);
        return 0;
    }
    let input = match invocation.input {
        Ok(command::Input::Document(raw)) => Ok(raw),
        Ok(command::Input::File(path)) => {
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
        Ok(raw) => execute_cancellable(client, &raw, &|| interrupt.requested()),
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
