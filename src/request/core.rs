//! Composition only: the operation's owner executes and produces its result.
use crate::remote::Error;
use crate::transport::Client;
use serde::Serialize;
use serde_json::Value;
pub(super) fn recognizes(operation: &str) -> bool {
    matches!(
        operation,
        "version"
            | "hosts"
            | "doctor"
            | "connection.status"
            | "session.create"
            | "session.list"
            | "session.exec"
            | "session.send"
            | "session.read"
            | "session.recover"
            | "session.close"
    )
}
fn json(value: impl Serialize) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|e| Error::new("INTERNAL", e.to_string()))
}
pub(super) fn execute(
    client: &Client,
    operation: &str,
    host: &str,
    params: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Value, Error> {
    match operation {
        "version" | "hosts" | "connection.status" => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Empty {}
            let _: Empty = serde_json::from_str(params)
                .map_err(|e| Error::new("CONFIG_INVALID", e.to_string()))?;
            crate::remote::Observation::new(client, cancelled).check()?;
            match operation {
                "version" => json(crate::wire::version()),
                "hosts" => json(crate::hosts::envelope(&crate::hosts::observe())),
                _ => {
                    let status = crate::connection::observe(client, host, cancelled)?;
                    json(crate::connection::connection(
                        "connection.status",
                        host,
                        &crate::connection::ConnectionDto::from(&status),
                    ))
                }
            }
        }
        "doctor" => {
            let options = crate::doctor::prepare(host, params)?;
            let probe = crate::doctor::execute(client, &options, cancelled)?;
            match &probe.failure {
                Some(outcome) => json(crate::doctor::doctor_failure(&probe.report, outcome)),
                None => json(crate::doctor::doctor(&probe.report)),
            }
        }
        _ => {
            let request = crate::session::prepare(operation, params)?;
            crate::session::execute(client, host, &request, cancelled)?
                .envelope(host)
                .map_err(|e| Error::new("INTERNAL", e.to_string()))
        }
    }
}
