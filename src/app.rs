//! Application use-cases.
pub mod doctor;
pub mod exec;
pub mod files;
pub(crate) mod remote;
pub mod session;

/// A use-case failure: the stable code an agent branches on, plus whether the
/// operation is safe to retry. English is for humans; the code is the contract
/// (AGENTS.md §6).
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
