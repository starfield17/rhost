//! Recoverable Runs and structured requests; no durable local registry.
pub mod command;
mod dto;
#[cfg(test)]
mod dto_tests;
mod request;
#[cfg(test)]
mod request_tests;
use crate::helper::Helper;
use crate::remote::Error;
use crate::transport::Client;
pub use command::{FLAGS, Invocation, parse};
use serde_json::Value;

pub fn execute_params(
    client: &Client,
    host: &str,
    operation: &str,
    params: &str,
    cancelled: &dyn Fn() -> bool,
    boot_guard: Option<&crate::domain::BootId>,
) -> Result<Value, Error> {
    let prepared = request::prepare(operation, host, params)?;
    let helper = Helper::new(client, &prepared.host)
        .with_cancellation(cancelled)
        .with_boot_guard(boot_guard);
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
    let mut identity = if prepared.kind != 8 {
        helper
            .ensure()
            .map_err(|e| e.with_recovery(crate::domain::Recovery::preparation()))?
    } else {
        helper
            .probe()
            .map_err(|e| e.with_recovery(crate::domain::Recovery::preparation()))?
    };
    let bytes = helper.call(prepared.kind, prepared.body, prepared.wait_ms)?;
    let mut data = dto::decode(prepared.kind, &bytes)?;
    if matches!(prepared.kind, 7 | 8) {
        if let Some(map) = identity.as_object_mut() {
            map.remove("boot_id");
        }
        data["helper"] = identity;
    }
    dto::bind(
        prepared.kind,
        &data,
        expected_generation.as_deref(),
        expected_key.as_deref(),
    )?;
    Ok(data)
}
