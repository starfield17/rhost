//! The schema-v6 wire envelope and the delivery it is written through.
//!
//! This is the shared half of the wire format: the envelope every operation
//! returns, the error payload every failure carries, the single-write delivery
//! that must never be retried once it fails (EXEC-010), and the two DTOs that
//! describe the CLI itself (`version`, `usage`). Per-capability DTOs live with
//! their capability and build on [`Envelope`].
//!
//! `wire/execution.rs` holds the one capability DTO that is genuinely shared:
//! the `exec` envelope is also what `doctor` reuses when a probe fails.

pub mod execution;

pub use execution::{exec, exec_diagnostic};

use serde::Serialize;
use std::io::{self, Write};

#[derive(Debug, Serialize)]
pub struct Envelope<T> {
    pub(crate) schema_version: u8,
    pub(crate) operation: &'static str,
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) host: Option<String>,
    pub(crate) data: T,
    pub(crate) error: Option<ErrorPayload>,
}

impl<T: Serialize> Envelope<T> {
    /// Delivery is attempted once. A broken sink cannot carry its own error
    /// envelope; the CLI reports that failure on stderr and exits nonzero.
    pub fn with_recovery(mut self, value: crate::domain::Recovery) -> Self {
        if let Some(e) = self.error.as_mut() {
            e.recovery = recovery(value);
        }
        self
    }
    pub fn write(&self, sink: &mut impl Write) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(self).map_err(io::Error::other)?;
        bytes.push(b'\n');
        sink.write_all(&bytes)
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorPayload {
    code: &'static str,
    message: String,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    mutation: Option<&'static str>,
    recovery: RecoveryDto,
}

#[derive(Debug, Serialize)]
pub struct RecoveryDto {
    stage: &'static str,
    submission: &'static str,
    action: &'static str,
}
pub fn recovery(value: crate::domain::Recovery) -> RecoveryDto {
    RecoveryDto {
        stage: value.stage(),
        submission: value.submission(),
        action: value.action(),
    }
}
impl ErrorPayload {
    pub fn with_recovery(mut self, value: crate::domain::Recovery) -> Self {
        self.recovery = recovery(value);
        self
    }
}

pub(crate) fn error(code: &'static str, message: &str, retryable: bool) -> ErrorPayload {
    ErrorPayload {
        code,
        message: message.into(),
        retryable,
        mutation: None,
        recovery: recovery(if matches!(code, "USAGE_ERROR" | "CONFIG_INVALID") {
            crate::domain::Recovery::invalid()
        } else {
            crate::domain::Recovery::default()
        }),
    }
}

/// The error payload for a use-case failure, so a code never has to be repeated
/// at each call site.
pub fn error_payload(code: &'static str, message: &str, retryable: bool) -> ErrorPayload {
    error(code, message, retryable)
}

/// One failure envelope for an operation whose data is not (yet) describable.
pub fn failure(
    operation: &'static str,
    host: &str,
    code: &'static str,
    message: &str,
    retryable: bool,
) -> Envelope<Option<()>> {
    failure_with_mutation(operation, host, code, message, retryable, None)
}

pub fn failure_with_mutation(
    operation: &'static str,
    host: &str,
    code: &'static str,
    message: &str,
    retryable: bool,
    mutation: Option<crate::domain::MutationEvidence>,
) -> Envelope<Option<()>> {
    let mut payload = error(code, message, retryable);
    payload.mutation = mutation
        .map(crate::domain::MutationEvidence::as_str)
        .or_else(|| matches!(operation, "fs.write" | "fs.patch").then_some("not_applied"));
    Envelope {
        schema_version: 6,
        operation,
        ok: false,
        host: (!host.is_empty()).then(|| host.to_string()),
        data: None,
        error: Some(payload),
    }
}

#[derive(Debug, Serialize)]
pub struct VersionDto {
    version: &'static str,
    commit: &'static str,
    build_date: &'static str,
    schema_version: u8,
}

pub fn version() -> Envelope<VersionDto> {
    Envelope {
        schema_version: 6,
        operation: "version",
        ok: true,
        host: None,
        error: None,
        data: VersionDto {
            version: env!("CARGO_PKG_VERSION"),
            commit: option_env!("RHOST_BUILD_COMMIT").unwrap_or("none"),
            build_date: option_env!("RHOST_BUILD_DATE").unwrap_or("unknown"),
            schema_version: 6,
        },
    }
}

pub fn usage(message: &str) -> Envelope<Option<()>> {
    Envelope {
        schema_version: 6,
        operation: "usage",
        ok: false,
        host: None,
        data: None,
        error: Some(error("USAGE_ERROR", message, false)),
    }
}

/// A command group usage refusal. Humans get the diagnostic plus help text when
/// missing a subcommand; `--json` gets exactly one envelope carrying the diagnostic,
/// because prose on stdout breaks an agent's decoder (AGENTS.md §6).
pub fn group_usage(group: &str, message: &str) -> Envelope<Option<()>> {
    let default_message = format!("rhost {group} needs a subcommand");
    let message = if message == default_message {
        format!("{default_message} (see: rhost {group} --help)")
    } else {
        message.to_string()
    };
    Envelope {
        schema_version: 6,
        operation: match group {
            "session" => "session.usage",
            "connection" => "connection.usage",
            "tunnel" => "tunnel.usage",
            _ => "fs.usage",
        },
        ok: false,
        host: None,
        data: None,
        error: Some(error("USAGE_ERROR", &message, false)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn active_result_fixtures_and_version_match_schema_six()
    -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::Value;
        let schema: Value =
            serde_json::from_str(include_str!("../../schemas/result-v6.schema.json"))?;
        let validator = jsonschema::validator_for(&schema)?;
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("../../tests/fixtures/result-v6-valid.json"))?;
        for fixture in fixtures {
            let result = &fixture["result"];
            assert!(validator.is_valid(result), "{result}");
            let mut old = result.clone();
            old["schema_version"] = 5.into();
            assert!(!validator.is_valid(&old), "{old}");
        }
        let version = serde_json::to_value(super::version())?;
        assert!(validator.is_valid(&version), "{version}");
        Ok(())
    }
}
