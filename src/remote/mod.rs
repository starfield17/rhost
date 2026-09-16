//! The shared remote execution substrate.
//!
//! Two things live here and nowhere else, because every capability needs them
//! and they must not be duplicated: [`Error`] (the stable `error.code` an agent
//! branches on, plus retryability) and [`run`]/[`exec`] (the one path that
//! submits a program to a host and the one place a transport failure is
//! classified). Capabilities depend on this module; it depends on none of them.

pub mod exec;
mod run;

pub(crate) use run::{RemoteRun, internal, run_remote, run_remote_partial};

/// A use-case failure: the stable code an agent branches on, plus whether the
/// operation is safe to retry.
#[derive(Debug, Clone)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: false,
        }
    }

    /// Marked only where retrying cannot compound a side effect.
    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}
