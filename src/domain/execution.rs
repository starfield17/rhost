use super::{DomainError, ExitCode, VerifiedCompletion};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Execution {
    Completed(ExitCode),
    Unknown,
    NotStarted,
}
#[derive(Debug, PartialEq, Eq)]
pub enum CompletionEvidence {
    Completed(VerifiedCompletion),
    Missing,
}
impl From<&CompletionEvidence> for Execution {
    fn from(value: &CompletionEvidence) -> Self {
        match value {
            CompletionEvidence::Completed(evidence) => Self::Completed(evidence.exit_code()),
            CompletionEvidence::Missing => Self::Unknown,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupEvidence {
    NotAttempted,
    ConfirmedStopped,
    Unconfirmed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelSignal {
    Int,
    Term,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interruption {
    Timeout,
    Cancelled(CancelSignal),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreExecFailure {
    Usage,
    Config,
    HostUnknown,
    Authentication,
    HostKey,
    Connection,
    DependencyMissing,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecFailure {
    Interrupted(Interruption),
    ExecutionUnknown,
    OutputWriteFailed,
    BeforeSubmission(PreExecFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedText {
    content: Vec<u8>,
    bytes: u64,
    truncated: bool,
}
impl CapturedText {
    /// A capture known to hold nothing: used before any submission exists.
    pub fn empty() -> Self {
        Self {
            content: Vec::new(),
            bytes: 0,
            truncated: false,
        }
    }
    /// Text construction for fixtures and already-decoded UTF-8 values.
    pub fn new(content: String, bytes: u64) -> Result<Self, DomainError> {
        Self::from_bytes(content.into_bytes(), bytes)
    }
    /// Preserve raw capture accounting even when JSON must replace invalid UTF-8.
    /// Human streaming remains byte-for-byte in the future transport layer.
    pub fn from_bytes(content: Vec<u8>, bytes: u64) -> Result<Self, DomainError> {
        if bytes < content.len() as u64 {
            return Err(DomainError("capture exceeds observed byte count"));
        }
        Ok(Self {
            truncated: bytes > content.len() as u64,
            content,
            bytes,
        })
    }
    pub fn content(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.content)
    }
    /// A capture whose text was *derived* from the raw bytes — terminal escapes
    /// stripped, invalid UTF-8 replaced — rather than copied from them.
    ///
    /// The two lengths are then not comparable: a projection can be shorter or
    /// longer than its source, so truncation is stated here instead of inferred
    /// from a mismatch (SESSION-008).
    pub fn projected(content: Vec<u8>, source_bytes: u64, truncated: bool) -> Self {
        Self {
            content,
            bytes: source_bytes,
            truncated,
        }
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: CapturedText,
    pub stderr: CapturedText,
}

impl ProcessOutput {
    pub fn empty() -> Self {
        Self {
            stdout: CapturedText::empty(),
            stderr: CapturedText::empty(),
        }
    }
}

/// Storage makes successful unknown execution unrepresentable.
/// ```compile_fail
/// let result = rhost::domain::ExecOutcome { execution: rhost::domain::Execution::Unknown };
/// ```
#[derive(Debug)]
pub struct ExecOutcome {
    state: ExecState,
    output: ProcessOutput,
    duration_ms: u64,
}
#[derive(Debug)]
enum ExecState {
    Completed(VerifiedCompletion),
    InterruptedUnknown {
        reason: Interruption,
        cleanup: CleanupEvidence,
    },
    InterruptedCompleted {
        reason: Interruption,
        completion: VerifiedCompletion,
    },
    Unknown,
    BeforeSubmission(PreExecFailure),
    OutputFailed {
        evidence: CompletionEvidence,
        cleanup: CleanupEvidence,
    },
}
impl ExecOutcome {
    pub fn completed(
        completion: VerifiedCompletion,
        output: ProcessOutput,
        duration_ms: u64,
    ) -> Self {
        Self {
            state: ExecState::Completed(completion),
            output,
            duration_ms,
        }
    }
    pub fn interrupted(
        evidence: CompletionEvidence,
        reason: Interruption,
        cleanup: CleanupEvidence,
        output: ProcessOutput,
        duration_ms: u64,
    ) -> Result<Self, DomainError> {
        let state = match (evidence, cleanup) {
            (CompletionEvidence::Completed(completion), CleanupEvidence::NotAttempted) => {
                ExecState::InterruptedCompleted { reason, completion }
            }
            (CompletionEvidence::Completed(_), _) => {
                return Err(DomainError(
                    "observed foreground completion must not trigger interruption cleanup",
                ));
            }
            (CompletionEvidence::Missing, cleanup) => {
                ExecState::InterruptedUnknown { reason, cleanup }
            }
        };
        Ok(Self {
            state,
            output,
            duration_ms,
        })
    }
    pub fn unknown(output: ProcessOutput, duration_ms: u64) -> Self {
        Self {
            state: ExecState::Unknown,
            output,
            duration_ms,
        }
    }
    pub fn not_started(reason: PreExecFailure, output: ProcessOutput, duration_ms: u64) -> Self {
        Self {
            state: ExecState::BeforeSubmission(reason),
            output,
            duration_ms,
        }
    }
    pub fn output_failed(
        evidence: CompletionEvidence,
        cleanup: CleanupEvidence,
        output: ProcessOutput,
        duration_ms: u64,
    ) -> Self {
        Self {
            state: ExecState::OutputFailed { evidence, cleanup },
            output,
            duration_ms,
        }
    }
    pub fn execution(&self) -> Execution {
        match &self.state {
            ExecState::Completed(completion)
            | ExecState::InterruptedCompleted { completion, .. } => {
                Execution::Completed(completion.exit_code())
            }
            ExecState::BeforeSubmission(_) => Execution::NotStarted,
            ExecState::OutputFailed { evidence, .. } => evidence.into(),
            ExecState::Unknown | ExecState::InterruptedUnknown { .. } => Execution::Unknown,
        }
    }
    pub fn failure(&self) -> Option<ExecFailure> {
        match &self.state {
            ExecState::Completed(_) => None,
            ExecState::InterruptedUnknown { reason, .. }
            | ExecState::InterruptedCompleted { reason, .. } => {
                Some(ExecFailure::Interrupted(*reason))
            }
            ExecState::BeforeSubmission(reason) => Some(ExecFailure::BeforeSubmission(*reason)),
            ExecState::Unknown => Some(ExecFailure::ExecutionUnknown),
            ExecState::OutputFailed { .. } => Some(ExecFailure::OutputWriteFailed),
        }
    }
    pub fn cleanup(&self) -> CleanupEvidence {
        match &self.state {
            ExecState::InterruptedUnknown { cleanup, .. }
            | ExecState::OutputFailed { cleanup, .. } => *cleanup,
            _ => CleanupEvidence::NotAttempted,
        }
    }
    pub fn output(&self) -> &ProcessOutput {
        &self.output
    }
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
    pub fn process_status(&self) -> u8 {
        match &self.state {
            ExecState::Completed(completion) => completion.exit_code().get(),
            ExecState::InterruptedUnknown { reason, .. }
            | ExecState::InterruptedCompleted { reason, .. } => match reason {
                Interruption::Timeout => 124,
                Interruption::Cancelled(CancelSignal::Int) => 130,
                Interruption::Cancelled(CancelSignal::Term) => 143,
            },
            _ => 255,
        }
    }
}
