//! Recoverable Runs and structured requests; no durable local registry.
pub mod command;
mod dto;
#[cfg(test)]
mod dto_tests;
mod follow;
mod logs;
mod request;
#[cfg(test)]
mod request_tests;
use crate::helper::Helper;
use crate::remote::Error;
use crate::transport::Client;
pub use command::{FLAGS, Invocation, parse};
use serde_json::Value;

pub(crate) fn execute_params(
    client: &Client,
    host: &str,
    operation: &str,
    params: &str,
    cancelled: &dyn Fn() -> bool,
    boot_guard: Option<&crate::domain::BootId>,
    pool: Option<&crate::helper::mux::Pool>,
) -> Result<Value, Error> {
    if matches!(operation, "run.watch" | "run.read" | "run.search") {
        return logs::execute(client, host, operation, params, cancelled, boot_guard, pool)
            .map_err(|mut e| {
                if e.recovery.stage() == "unknown" {
                    e.recovery = crate::domain::Recovery::observe(
                        crate::domain::Phase::Observation,
                        crate::domain::Submission::MayHaveSubmitted,
                    );
                }
                e
            });
    }
    if operation == "run.list" || operation.starts_with("state.") {
        let helper = Helper::new(client, host)
            .with_cancellation(cancelled)
            .with_boot_guard(boot_guard)
            .with_pool(pool);
        return crate::helper::registry::execute(&helper, operation, params);
    }
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
    // Keep the RH01 default start path and its historical key fingerprint.
    let mux = !matches!(prepared.kind, 7 | 8)
        && (prepared.kind != 2 || prepared.cgroup || pool.is_some());
    let reply = if mux {
        helper
            .mux(
                pool,
                prepared.kind,
                prepared.body,
                prepared.wait_ms,
                |_, _| Err(crate::helper::codec::protocol("unexpected Run event")),
            )
            .map(|bytes| (Value::Null, bytes))
    } else {
        helper.operation(prepared.kind, prepared.body, prepared.wait_ms)
    };
    let (mut identity, bytes) = reply.map_err(|mut error| {
        use crate::domain::{Phase, Recovery, Submission};
        if error.recovery.stage() == "unknown" {
            error.recovery = if prepared.kind == 2 {
                Recovery::same_request(Phase::Submission, Submission::MayHaveSubmitted)
            } else {
                Recovery::observe(Phase::Observation, Submission::MayHaveSubmitted)
            };
        }
        error
    })?;
    let mut data = dto::decode(prepared.kind, &bytes, mux)?;
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
