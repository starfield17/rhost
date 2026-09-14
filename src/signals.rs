//! Local interruption: the two signals a caller uses to stop an invocation.
//!
//! Registering handlers is the one place this crate touches process signals. It
//! records *that* a signal arrived and *which* one, so the reported reason and
//! the process status (130 vs 143) come from what actually happened rather than
//! from a guess made afterwards (WIRE-004).
use crate::domain::{CancelSignal, Interruption};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Default)]
pub struct Interrupt {
    interrupted: Arc<AtomicBool>,
    term: Arc<AtomicBool>,
}

impl Interrupt {
    /// Installs the handlers for the lifetime of the process. There is no
    /// unregistering: a CLI process that stops wanting interruption has stopped.
    pub fn install() -> Result<Self, std::io::Error> {
        let interrupted = Arc::new(AtomicBool::new(false));
        let term = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGINT, interrupted.clone())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        signal_hook::flag::register(signal_hook::consts::SIGTERM, interrupted.clone())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        signal_hook::flag::register(signal_hook::consts::SIGTERM, term.clone())
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(Self { interrupted, term })
    }

    pub fn requested(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }

    /// The signal that arrived, if any. `SIGTERM` is distinguished from
    /// `SIGINT`; anything else is reported as a cancellation without pretending
    /// to know which.
    pub fn signal(&self) -> Option<CancelSignal> {
        if !self.requested() {
            return None;
        }
        Some(if self.term.load(Ordering::SeqCst) {
            CancelSignal::Term
        } else {
            CancelSignal::Int
        })
    }

    pub fn interruption(&self) -> Option<Interruption> {
        self.signal().map(Interruption::Cancelled)
    }
}
