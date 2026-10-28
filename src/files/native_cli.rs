//! Convenience argv for the same closed file parameter documents used by MCP.
use crate::{cli::Sink, remote::Error, transport::Client};
use std::io::Read;
pub(super) struct Options<'a> {
    pub host: &'a str,
    pub operation: &'a str,
    pub from: &'a str,
    pub guard: Option<&'a str>,
    pub json: bool,
}
pub(super) fn run(
    sink: &mut Sink,
    client: &Client,
    options: Options<'_>,
    cancelled: &dyn Fn() -> bool,
) -> u8 {
    let Options {
        host,
        operation,
        from,
        guard,
        json,
    } = options;
    let result = (|| {
        let guard = guard
            .map(|s| crate::domain::BootId::new(s.into()))
            .transpose()
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        let input: Box<dyn Read> = if from == "-" {
            Box::new(std::io::stdin())
        } else {
            Box::new(
                std::fs::File::open(from)
                    .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?,
            )
        };
        let mut bytes = Vec::new();
        input
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(Error::new(
                "FILE_TOO_LARGE",
                "parameter document exceeds 16 MiB",
            ));
        }
        let raw = String::from_utf8(bytes)
            .map_err(|_| Error::new("CONFIG_INVALID", "parameters must be UTF-8 JSON"))?;
        Ok(super::execute_params(
            client,
            host,
            operation,
            &raw,
            cancelled,
            guard.as_ref(),
        ))
    })();
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => super::RequestOutcome::failed(error),
    };
    let status = if outcome.error.is_none() { 0 } else { 255 };
    let value = outcome.envelope(operation, host);
    match if json {
        serde_json::to_string(&value)
    } else {
        serde_json::to_string_pretty(&value)
    } {
        Ok(text) => sink.line(&text),
        Err(e) => {
            crate::cli::warn(&format!("rhost: OUTPUT_WRITE_FAILED: {e}"));
            return 255;
        }
    }
    status
}
