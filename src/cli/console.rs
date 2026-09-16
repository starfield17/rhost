//! Delivering a result on stdout, and reporting a lost one.
//!
//! Delivery is attempted once. A sink that stops accepting bytes is a delivery
//! failure, never a rerun: the operation may already have completed (EXEC-010).

use super::Delivery;
use crate::stdio::Stdout;
use crate::wire::{Envelope, failure};
use serde::Serialize;
use std::io::{self, Write};

/// A stdout writer that remembers its first failure instead of panicking.
///
/// `print!` aborts the process when the pipe is closed, and a half-written
/// document on stdout is exactly what an agent must not be left to decode. Every
/// write is followed by a flush, so bytes forwarded by a child process and bytes
/// written here cannot overtake each other.
pub struct Sink {
    out: Stdout,
    failed: Option<io::Error>,
}

impl Default for Sink {
    fn default() -> Self {
        Self::new()
    }
}

impl Sink {
    /// One stdout handle per delivery: every write goes through a duplicate of
    /// descriptor 1, so an unwritable sink is a failure rather than a silent
    /// success (EXEC-010).
    pub fn new() -> Self {
        Self {
            out: Stdout::open(),
            failed: None,
        }
    }

    pub fn bytes(&mut self, data: &[u8]) {
        if self.failed.is_some() {
            return;
        }
        let stdout = &mut self.out;
        self.failed = stdout.write_all(data).and_then(|()| stdout.flush()).err();
    }

    pub fn text(&mut self, text: &str) {
        self.bytes(text.as_bytes());
    }

    pub fn line(&mut self, text: &str) {
        self.bytes(text.as_bytes());
        self.bytes(b"\n");
    }

    pub fn envelope<T: Serialize>(&mut self, document: &Envelope<T>) {
        if self.failed.is_some() {
            return;
        }
        let stdout = &mut self.out;
        self.failed = document.write(stdout).and_then(|()| stdout.flush()).err();
    }

    /// Closes the delivery. A lost stdout is reported on stderr — the only
    /// channel left — with a status that says the outcome is unknown to the
    /// caller, not that the operation failed (EXEC-010).
    pub fn finish(self, status: u8) -> Delivery {
        match self.failed {
            None => Delivery {
                status,
                delivery_failed: false,
            },
            Some(error) => {
                warn(&format!(
                    "rhost: {}: result delivery failed (operation may have completed): {error}",
                    "OUTPUT_WRITE_FAILED"
                ));
                Delivery {
                    status: 255,
                    delivery_failed: true,
                }
            }
        }
    }
}

/// Writes one human-facing line to stderr. Diagnostics that cannot be written
/// have nowhere left to go, so their failure is deliberately not propagated.
pub fn warn(text: &str) {
    let mut stderr = io::stderr().lock();
    let _ = stderr
        .write_all(text.as_bytes())
        .and_then(|()| stderr.write_all(b"\n"))
        .and_then(|()| stderr.flush());
}

/// A failure with no operation data: the same text on both delivery paths, so a
/// human and an agent cannot be told different things (AGENTS.md §6).
pub struct Failure {
    operation: &'static str,
    host: String,
    code: &'static str,
    message: String,
    retryable: bool,
}

impl Failure {
    pub fn new(
        operation: &'static str,
        host: &str,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation,
            host: host.to_string(),
            code,
            message: message.into(),
            retryable: false,
        }
    }

    /// A use-case failure carries its own code and retryability, so the two paths
    /// cannot drift.
    pub fn from_error(operation: &'static str, host: &str, error: crate::remote::Error) -> Self {
        Self {
            operation,
            host: host.to_string(),
            code: error.code,
            message: error.message,
            retryable: error.retryable,
        }
    }

    /// Marked only where retrying cannot compound a side effect: a fault that
    /// happened before anything was built, or one whose cause is transport-shaped.
    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    /// The one mapping from the error taxonomy to a process status: 124 for a
    /// timeout, 255 for every other adapter failure, so a status in 0-254 is
    /// always the remote command's own (WIRE-004).
    pub fn status(&self) -> u8 {
        if self.code == "REMOTE_COMMAND_TIMEOUT" {
            124
        } else {
            255
        }
    }

    /// The code an agent branches on, for callers that also record the failure
    /// somewhere else (the audit trail).
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn deliver(self, sink: &mut Sink, json: bool) -> u8 {
        let status = self.status();
        if json {
            sink.envelope(&failure(
                self.operation,
                &self.host,
                self.code,
                &self.message,
                self.retryable,
            ));
        } else {
            warn(&format!("rhost: {}: {}", self.code, self.message));
        }
        status
    }
}

/// Renders a field table with the same column width the original CLI used.
pub fn field(sink: &mut Sink, key: &str, value: &str) {
    sink.line(&format!("{key:<18} {value}"));
}

pub fn yes(no: bool) -> &'static str {
    if no { "yes" } else { "no" }
}

pub fn ok_missing(found: bool) -> &'static str {
    if found { "OK" } else { "missing" }
}
