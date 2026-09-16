//! The schema-v2 wire envelope and the delivery it is written through.
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
}

pub(crate) fn error(code: &'static str, message: &str, retryable: bool) -> ErrorPayload {
    ErrorPayload {
        code,
        message: message.into(),
        retryable,
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
    Envelope {
        schema_version: 2,
        operation,
        ok: false,
        host: (!host.is_empty()).then(|| host.to_string()),
        data: None,
        error: Some(error(code, message, retryable)),
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
        schema_version: 2,
        operation: "version",
        ok: true,
        host: None,
        error: None,
        data: VersionDto {
            version: env!("CARGO_PKG_VERSION"),
            commit: option_env!("RHOST_BUILD_COMMIT").unwrap_or("none"),
            build_date: option_env!("RHOST_BUILD_DATE").unwrap_or("unknown"),
            schema_version: 2,
        },
    }
}

pub fn usage(message: &str) -> Envelope<Option<()>> {
    Envelope {
        schema_version: 2,
        operation: "usage",
        ok: false,
        host: None,
        data: None,
        error: Some(error("USAGE_ERROR", message, false)),
    }
}

/// A command group invoked without a subcommand. Humans get the group's help
/// text; `--json` gets exactly one envelope, because prose on stdout breaks an
/// agent's decoder (AGENTS.md §6).
pub fn group_usage(group: &str) -> Envelope<Option<()>> {
    let message = format!("rhost {group} needs a subcommand (see: rhost {group} --help)");
    Envelope {
        schema_version: 2,
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
