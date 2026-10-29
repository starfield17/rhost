//! One shared probe and audit outcome for CLI, JSON request and MCP.
use super::app::{self, Options, Probe};
use crate::audit::Timer;
use crate::remote::{Error, Observation};
use crate::transport::Client;
use serde::Deserialize;
use std::time::Duration;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    #[serde(default)]
    timeout_ms: u64,
    #[serde(default)]
    fresh: bool,
}
pub(crate) fn prepare<'a>(host: &'a str, raw: &str) -> Result<Options<'a>, Error> {
    let p: Params =
        serde_json::from_str(raw).map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
    if p.timeout_ms > 9_223_372_036_854 {
        return Err(Error::new(
            "CONFIG_INVALID",
            "timeout exceeds duration range",
        ));
    }
    Ok(Options {
        host,
        timeout: if p.timeout_ms == 0 {
            Duration::from_nanos(super::DEFAULT_TIMEOUT_NANOS as u64)
        } else {
            Duration::from_millis(p.timeout_ms)
        },
        fresh: p.fresh,
    })
}
pub(crate) fn execute(
    client: &Client,
    options: &Options<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Probe, Error> {
    Observation::new(client, cancelled).check()?;
    let timer = Timer::start("doctor", options.host);
    let result = app::probe_observed(client, options, cancelled)
        .map_err(|e| Error::new(e.code(), e.to_string()));
    match &result {
        Err(error) => timer.failed(error.code),
        Ok(probe) => match &probe.failure {
            Some(outcome) => timer.failed(
                crate::wire::exec_diagnostic(outcome)
                    .map(|v| v.0)
                    .unwrap_or("REMOTE_EXECUTION_UNKNOWN"),
            ),
            None => timer.succeeded("", "", None),
        },
    }
    result
}
