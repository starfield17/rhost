use super::{
    CapturedText, CompletionEvidence, Execution, SessionId, SessionRef, VerifiedCompletion,
};

/// Terminal output has no independent stderr channel.
#[derive(Debug)]
pub struct PtyOutput(pub CapturedText);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionFailure {
    Busy,
    WriterBusy,
    Timeout,
    Unhealthy,
    NotFound,
}
#[derive(Debug)]
pub struct SessionExecOutcome {
    state: SessionState,
    reference: SessionRef,
    output: PtyOutput,
}
#[derive(Debug)]
enum SessionState {
    Resolved {
        id: SessionId,
        outcome: ResolvedOutcome,
    },
    Unresolved(UnresolvedFailure),
}
#[derive(Debug)]
enum ResolvedOutcome {
    Completed(VerifiedCompletion),
    /// The pane is owned by a program instead of the managed shell. `foreground`
    /// is what the helper observed holding it, when it named one: the caller's
    /// next move is to drive or interrupt *that*, so the name is evidence rather
    /// than a detail.
    Busy {
        foreground: Option<String>,
    },
    WriterBusy,
    TimedOut {
        evidence: CompletionEvidence,
        preserved: bool,
    },
    Unhealthy,
}
#[derive(Debug)]
enum UnresolvedFailure {
    NotFound,
    Unhealthy,
}
impl SessionExecOutcome {
    fn resolved(
        id: SessionId,
        reference: SessionRef,
        outcome: ResolvedOutcome,
        output: PtyOutput,
    ) -> Self {
        Self {
            state: SessionState::Resolved { id, outcome },
            reference,
            output,
        }
    }
    pub fn completed(
        id: SessionId,
        reference: SessionRef,
        completion: VerifiedCompletion,
        output: PtyOutput,
    ) -> Self {
        Self::resolved(
            id,
            reference,
            ResolvedOutcome::Completed(completion),
            output,
        )
    }
    pub fn busy(id: SessionId, reference: SessionRef, output: PtyOutput) -> Self {
        Self::busy_in(id, reference, None, output)
    }
    /// The same refusal, naming what owns the pane.
    pub fn busy_in(
        id: SessionId,
        reference: SessionRef,
        foreground: Option<String>,
        output: PtyOutput,
    ) -> Self {
        Self::resolved(id, reference, ResolvedOutcome::Busy { foreground }, output)
    }
    pub fn writer_busy(id: SessionId, reference: SessionRef, output: PtyOutput) -> Self {
        Self::resolved(id, reference, ResolvedOutcome::WriterBusy, output)
    }
    pub fn timed_out(
        id: SessionId,
        reference: SessionRef,
        evidence: CompletionEvidence,
        preserved: bool,
        output: PtyOutput,
    ) -> Self {
        Self::resolved(
            id,
            reference,
            ResolvedOutcome::TimedOut {
                evidence,
                preserved,
            },
            output,
        )
    }
    pub fn unhealthy(id: Option<SessionId>, reference: SessionRef, output: PtyOutput) -> Self {
        match id {
            Some(id) => Self::resolved(id, reference, ResolvedOutcome::Unhealthy, output),
            None => Self {
                state: SessionState::Unresolved(UnresolvedFailure::Unhealthy),
                reference,
                output,
            },
        }
    }
    pub fn not_found(reference: SessionRef, output: PtyOutput) -> Self {
        Self {
            state: SessionState::Unresolved(UnresolvedFailure::NotFound),
            reference,
            output,
        }
    }
    pub fn id(&self) -> Option<&SessionId> {
        match &self.state {
            SessionState::Resolved { id, .. } => Some(id),
            SessionState::Unresolved(_) => None,
        }
    }
    pub fn reference(&self) -> &SessionRef {
        &self.reference
    }
    pub fn execution(&self) -> Execution {
        match &self.state {
            SessionState::Resolved {
                outcome: ResolvedOutcome::Completed(completion),
                ..
            } => Execution::Completed(completion.exit_code()),
            SessionState::Resolved {
                outcome: ResolvedOutcome::TimedOut { evidence, .. },
                ..
            } => evidence.into(),
            SessionState::Unresolved(UnresolvedFailure::Unhealthy)
            | SessionState::Resolved {
                outcome: ResolvedOutcome::Unhealthy,
                ..
            } => Execution::Unknown,
            _ => Execution::NotStarted,
        }
    }
    pub fn failure(&self) -> Option<SessionFailure> {
        match &self.state {
            SessionState::Unresolved(UnresolvedFailure::NotFound) => Some(SessionFailure::NotFound),
            SessionState::Unresolved(UnresolvedFailure::Unhealthy) => {
                Some(SessionFailure::Unhealthy)
            }
            SessionState::Resolved { outcome, .. } => match outcome {
                ResolvedOutcome::Completed(_) => None,
                ResolvedOutcome::Busy { .. } => Some(SessionFailure::Busy),
                ResolvedOutcome::WriterBusy => Some(SessionFailure::WriterBusy),
                ResolvedOutcome::TimedOut { .. } => Some(SessionFailure::Timeout),
                ResolvedOutcome::Unhealthy => Some(SessionFailure::Unhealthy),
            },
        }
    }
    pub fn output(&self) -> &PtyOutput {
        &self.output
    }
    /// The pane's foreground command, when a busy refusal named it.
    pub fn foreground(&self) -> Option<&str> {
        match &self.state {
            SessionState::Resolved {
                outcome: ResolvedOutcome::Busy { foreground },
                ..
            } => foreground.as_deref(),
            _ => None,
        }
    }
    pub fn preserved(&self) -> bool {
        match &self.state {
            SessionState::Resolved {
                outcome: ResolvedOutcome::Completed(_),
                ..
            } => true,
            SessionState::Resolved {
                outcome: ResolvedOutcome::TimedOut { preserved, .. },
                ..
            } => *preserved,
            _ => false,
        }
    }
}
