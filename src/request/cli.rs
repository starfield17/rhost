//! Stdin/file loading and one-document CLI delivery.
use super::{execute, failure};
use crate::remote::Error;
use crate::transport::Client;
use std::io::Read;
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
