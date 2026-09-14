//! Pure values and state transitions. No wire, process, or filesystem access.
mod execution;
mod files;
mod identity;
mod session;

pub use execution::{
    CancelSignal, CapturedText, CleanupEvidence, CompletionEvidence, ExecFailure, ExecOutcome,
    Execution, Interruption, PreExecFailure, ProcessOutput,
};
pub use files::{FileWrite, Sha256};
pub use identity::{
    DomainError, ExitCode, InvocationToken, RemotePath, SessionId, SessionRef, VerifiedCompletion,
};
pub use session::{PtyOutput, SessionExecOutcome, SessionFailure};
