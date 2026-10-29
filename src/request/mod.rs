//! Delivery-neutral composition for CLI JSON and MCP; capability types stay owned.
use crate::remote::Error;
use crate::transport::Client;
use serde_json::{Value, json};

mod core;
mod input;
pub(crate) use input::{Call, arguments as from_arguments};
pub fn execute(client: &Client, raw: &str, cancelled: &dyn Fn() -> bool) -> Value {
    match input::raw(raw) {
        Ok(call) => execute_call(client, call, cancelled),
        Err(error) => failure("request", None, error),
    }
}
pub(crate) fn execute_call(client: &Client, q: Call, cancelled: &dyn Fn() -> bool) -> Value {
    if core::recognizes(&q.operation) {
        return match core::execute(
            client,
            &q.operation,
            q.host.as_deref().unwrap_or(""),
            q.params.get(),
            cancelled,
        ) {
            Ok(value) => value,
            Err(error) => failure(&q.operation, q.host.as_deref(), error),
        };
    }
    let host = q.host.as_deref().unwrap_or("");
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
            host,
            &q.operation,
            q.params.get(),
            cancelled,
            guard.as_ref(),
        );
        return outcome.envelope(&q.operation, host);
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
            host,
            &q.operation,
            q.params.get(),
            cancelled,
            guard.as_ref(),
        )
    } else {
        return failure(
            "request",
            Some(host),
            Error::new("USAGE_ERROR", "unknown structured operation"),
        );
    };
    match result {
        Ok(data) => {
            json!({"schema_version":4,"operation":q.operation,"ok":true,"host":host,"data":data,"error":null})
        }
        Err(e) => failure(&q.operation, Some(host), e),
    }
}
pub fn failure(operation: &str, host: Option<&str>, error: Error) -> Value {
    let mut detail = json!({"code":error.code,"message":error.message,"retryable":error.retryable,"recovery":crate::wire::recovery(error.recovery)});
    if let Some(mutation) = error.mutation {
        detail["mutation"] = mutation.as_str().into();
    } else if matches!(operation, "fs.write" | "fs.patch") {
        detail["mutation"] = "not_applied".into();
    }
    let mut envelope = json!({"schema_version":4,"operation":operation,"ok":false,"host":host,"data":null,"error":detail});
    if host.is_none() && core::recognizes(operation) {
        if let Some(object) = envelope.as_object_mut() {
            object.remove("host");
        }
    }
    envelope
}
pub fn invalid(message: impl Into<String>) -> Value {
    failure(
        "request",
        None,
        Error::new("CONFIG_INVALID", message).with_recovery(crate::domain::Recovery::invalid()),
    )
}
pub fn observation_cancelled(operation: &str, host: Option<&str>) -> Value {
    failure(
        operation,
        host,
        Error::new(
            "REMOTE_COMMAND_CANCELLED",
            "local observation cancelled before submission",
        )
        .with_recovery(crate::domain::Recovery::not_dispatched()),
    )
}
pub fn busy(operation: &str, host: Option<&str>) -> Value {
    failure(
        operation,
        host,
        Error::new("MCP_BUSY", "bounded MCP admission queue is full")
            .retryable()
            .with_recovery(crate::domain::Recovery::not_dispatched()),
    )
}

mod cli;
pub use cli::run;
