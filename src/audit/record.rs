//! Shared fail-open audit recording; no CLI rendering dependency.
use super::trail;
use crate::config;
use std::io::Write;
use std::time::Instant;
fn warn(message: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{message}");
}

/// One operation, timed, and written to the trail when the caller says how it
/// ended. A recorder that cannot write never fails the operation.
pub struct Timer {
    start: Instant,
    recorder: trail::Recorder,
    operation: &'static str,
    host: String,
}

impl Timer {
    pub fn start(operation: &'static str, host: &str) -> Self {
        Self {
            start: Instant::now(),
            recorder: trail::Recorder::from_env(&config::v4_state_dir()),
            operation,
            host: host.to_string(),
        }
    }

    /// Records an operation that completed as designed. `exit_code` is the remote
    /// command's own status when it has one, and `None` for operations whose
    /// answer is not a status.
    pub fn succeeded(&self, cwd: &str, command_summary: &str, exit_code: Option<u8>) {
        self.write(&trail::Entry::completed(
            self.operation,
            &self.host,
            cwd,
            command_summary,
            exit_code,
            self.elapsed_ms(),
        ));
    }

    /// Records a refusal or a failure, keeping the code an agent would branch on.
    pub fn failed(&self, error_code: &str) {
        self.write(&trail::Entry::refused(
            self.operation,
            &self.host,
            error_code,
            self.elapsed_ms(),
        ));
    }

    fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn write(&self, entry: &trail::Entry) {
        if let Err(error) = self.recorder.record(entry) {
            warn(&format!("rhost: audit: {error}"));
        }
    }
}

/// Records one entry the caller has already timed and described, used where one
/// command covers several operations (a batch) and each one has its own duration.
pub fn record(entry: trail::Entry) {
    let recorder = trail::Recorder::from_env(&config::v4_state_dir());
    if let Err(error) = recorder.record(&entry) {
        warn(&format!("rhost: audit: {error}"));
    }
}
