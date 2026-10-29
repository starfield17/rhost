//! Pure values and state transitions. No wire, process, or filesystem access.
mod context;
mod execution;
pub use context::BootId;
mod files;
mod identity;
mod mutation;
mod recovery;
pub use recovery::{Phase, Recovery, Submission};
mod session;

pub use execution::{
    CancelSignal, CapturedText, CleanupEvidence, CompletionEvidence, ExecFailure, ExecOutcome,
    Execution, Interruption, PreExecFailure, ProcessOutput,
};
pub use files::{FileWrite, Sha256};
pub use identity::{
    DomainError, ExitCode, InvocationToken, RemotePath, SessionId, SessionRef, VerifiedCompletion,
};
pub use mutation::MutationEvidence;
pub use session::{PtyOutput, SessionExecOutcome, SessionFailure};
