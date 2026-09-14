//! Running the local transfer tools as owned children.
//!
//! `scp` and `rsync` are short-lived processes the CLI starts and must be able
//! to stop. They run in their own process group so a tool that forked a wrapper
//! cannot keep the transfer's pipes open after the tool itself is gone, and a
//! deadline is a deadline rather than an abandoned copy (FS-005).

use super::split_remote_spec;
use crate::domain::CancelSignal;
use crate::shell;
use crate::transport::process::{self, Keep, Spec, StdinSource, Tap};
use std::io;
use std::time::{Duration, Instant};

/// Bound on one tool's captured output. A plan for a very large tree is a real
/// answer the caller asked for, so this is generous; exceeding it is reported
/// rather than silently truncating a plan.
const OUTPUT_LIMIT: usize = 64 * 1024 * 1024;

/// The budget for one transfer when the caller names none. A copy's duration
/// belongs to the size of the data, not to the command.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The `rsync --version` probe is only used to choose how a remote path is
/// quoted. Cutting it short silently takes the conservative route, which is
/// correct but slower, so the bound is generous.
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// How a stopped run learns it should stop, and which signal ended it.
#[derive(Clone, Copy)]
pub struct Stop<'a> {
    pub requested: &'a dyn Fn() -> bool,
    pub signal: &'a dyn Fn() -> Option<CancelSignal>,
}

fn never_requested() -> bool {
    false
}

fn never_cancelled() -> Option<CancelSignal> {
    None
}

impl Stop<'_> {
    /// A run nobody may stop locally: helper probes and other unattended work.
    pub fn none() -> Self {
        Self {
            requested: &never_requested,
            signal: &never_cancelled,
        }
    }
}

/// The tool names rhost runs. They are fields rather than constants so a test
/// can prove argument construction against a stub.
pub struct Tools {
    pub scp: String,
    pub rsync: String,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            scp: "scp".to_string(),
            rsync: "rsync".to_string(),
        }
    }
}

/// One finished tool invocation. A missing exit status is never zero.
pub struct Outcome {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit: Option<i32>,
    pub timed_out: bool,
    pub cancelled: Option<CancelSignal>,
    pub duration: Duration,
    /// The tool could not be started at all — typically: not installed here.
    pub spawn_error: Option<io::Error>,
    /// The tool produced more output than the capture bound holds.
    pub overflow: bool,
}

impl Outcome {
    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_string()
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    /// The first line of the tool's own complaint, or its stdout when stderr is
    /// silent: "scp exited 1" tells an agent nothing it can act on.
    pub fn diagnostic(&self) -> String {
        let first = |text: &str| {
            text.replace("\r\n", "\n")
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or("")
                .to_string()
        };
        let message = first(&self.stderr_text());
        if !message.is_empty() {
            return clip(&message, 300);
        }
        let message = first(&self.stdout_text());
        if message.is_empty() {
            "no diagnostic".to_string()
        } else {
            clip(&message, 300)
        }
    }
}

/// Bounds a diagnostic taken from a subprocess at a rune boundary.
fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut cut = limit;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}...", &text[..cut])
}

/// Runs one local tool with a deadline, its own process group, and bounded
/// capture. A tool that is not installed is reported as such, never as a
/// remote failure.
pub fn run(program: &str, args: &[String], timeout: Duration, stop: Stop<'_>) -> Outcome {
    let mut stdout = Tap::new(None, OUTPUT_LIMIT, Keep::Prefix);
    let mut stderr = Tap::new(None, OUTPUT_LIMIT, Keep::Prefix);
    let started = Instant::now();
    let run = process::run(
        &Spec {
            program,
            args,
            // A tool that asks for a password would hang a batch invocation:
            // authentication comes from keys or an agent (AGENTS.md §5).
            stdin: StdinSource::Closed,
            deadline: Some(
                started
                    + if timeout.is_zero() {
                        DEFAULT_TIMEOUT
                    } else {
                        timeout
                    },
            ),
            cancelled: stop.requested,
            group: true,
        },
        &mut stdout,
        &mut stderr,
    );
    let cancelled = run
        .cancelled
        .then(|| (stop.signal)().unwrap_or(CancelSignal::Term));
    Outcome {
        spawn_error: match run.failure {
            Some(process::RunFailure::Spawn(error)) => Some(error),
            Some(process::RunFailure::Stream(error)) => Some(error),
            None => None,
        },
        stdout: stdout.body(),
        stderr: stderr.body(),
        exit: run.exit,
        timed_out: run.timed_out,
        cancelled,
        duration: run.duration,
        overflow: stdout.total() > OUTPUT_LIMIT as u64 || stderr.total() > OUTPUT_LIMIT as u64,
    }
}

impl Tools {
    /// Prepares an rsync argv so the remote path survives the remote login
    /// shell. GNU rsync 3 understands `--protect-args`; openrsync and rsync 2
    /// do not, and would fail the whole transfer on an unrecognised option, so
    /// the path is quoted for the remote shell by hand instead.
    pub fn safe_rsync_paths(&self, args: &[String], stop: Stop<'_>) -> Vec<String> {
        let needy =
            (args.len().saturating_sub(2)..args.len()).find(|index| {
                match split_remote_spec(&args[*index]) {
                    Some((_, path)) => super::path_needs_quoting(path),
                    None => false,
                }
            });
        let Some(index) = needy else {
            return args.to_vec();
        };
        if self.protect_args_supported(stop) {
            let mut out = vec!["--protect-args".to_string()];
            out.extend_from_slice(args);
            return out;
        }
        let mut out = args.to_vec();
        if let Some((host, path)) = split_remote_spec(&args[index]) {
            out[index] = super::remote_spec(host, &shell::path_quote(path));
        }
        out
    }

    fn protect_args_supported(&self, stop: Stop<'_>) -> bool {
        let outcome = run(
            &self.rsync,
            &["--version".to_string()],
            VERSION_PROBE_TIMEOUT,
            stop,
        );
        if outcome.exit != Some(0) {
            return false;
        }
        protect_args_decision(&outcome.stdout_text())
    }
}

/// Given a `--version` banner, does this tool understand `--protect-args`?
fn protect_args_decision(stdout: &str) -> bool {
    let first = stdout.lines().next().unwrap_or("").trim();
    if first.to_lowercase().contains("openrsync") {
        return false;
    }
    // The banner is `rsync  version 3.2.7  protocol version 31`: the word
    // `version` has to follow `rsync` directly, which is what keeps openrsync's
    // own banner out of this branch.
    let mut words = first.split_whitespace();
    if !words
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("rsync"))
        || !words
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("version"))
    {
        return false;
    }
    let major: String = words
        .next()
        .unwrap_or("")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    major.parse::<u32>().is_ok_and(|major| major >= 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_gnu_rsync_three_and_later_understand_protect_args() {
        assert!(protect_args_decision(
            "rsync  version 3.2.7  protocol version 31\nfeatures"
        ));
        assert!(protect_args_decision(
            "rsync  version 3.0.9  protocol version 30"
        ));
        assert!(!protect_args_decision(
            "rsync  version 2.6.9  protocol version 29"
        ));
        assert!(!protect_args_decision("openrsync: protocol version 29"));
        assert!(!protect_args_decision("rsync not found"));
        assert!(!protect_args_decision(""));
    }
}
