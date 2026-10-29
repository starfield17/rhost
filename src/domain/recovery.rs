//! Recovery advice records evidence; it never authorizes replaying unknown work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Validation,
    Preparation,
    Submission,
    Observation,
    Operation,
    Unknown,
}
impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Preparation => "preparation",
            Self::Submission => "submission",
            Self::Observation => "observation",
            Self::Operation => "operation",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    NotSubmitted,
    MayHaveSubmitted,
    Confirmed,
}
impl Submission {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotSubmitted => "not_submitted",
            Self::MayHaveSubmitted => "may_have_submitted",
            Self::Confirmed => "confirmed",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recovery {
    phase: Phase,
    submission: Submission,
    action: &'static str,
}
impl Default for Recovery {
    fn default() -> Self {
        Self::inspect(Phase::Unknown, Submission::MayHaveSubmitted)
    }
}
impl Recovery {
    pub fn invalid() -> Self {
        Self {
            phase: Phase::Validation,
            submission: Submission::NotSubmitted,
            action: "correct_request",
        }
    }
    pub fn preparation() -> Self {
        Self {
            phase: Phase::Preparation,
            submission: Submission::NotSubmitted,
            action: "retry_request",
        }
    }
    pub fn not_dispatched() -> Self {
        Self {
            phase: Phase::Submission,
            submission: Submission::NotSubmitted,
            action: "retry_request",
        }
    }
    pub fn inspect(phase: Phase, submission: Submission) -> Self {
        Self {
            phase,
            submission,
            action: "inspect_state",
        }
    }
    pub fn observe(phase: Phase, submission: Submission) -> Self {
        Self {
            phase,
            submission,
            action: "retry_observation",
        }
    }
    /// The caller must already own the unchanged generation/key/intent.
    pub fn same_request(phase: Phase, submission: Submission) -> Self {
        Self {
            phase,
            submission,
            action: "retry_same_request",
        }
    }
    pub fn stage(self) -> &'static str {
        self.phase.as_str()
    }
    pub fn submission(self) -> &'static str {
        self.submission.as_str()
    }
    pub fn action(self) -> &'static str {
        self.action
    }
}
