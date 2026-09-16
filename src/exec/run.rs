//! Running `exec`: one foreground program, forwarded as it is produced.
//!
//! This is the one command that streams live output, so it owns the choice of
//! forward sinks and the local interruption wiring. What a finished run *means*
//! is decided in `crate::remote`; this file turns the request into a run and
//! delivers the answer.

use super::{Exec, MAX_COMMAND_FILE_BYTES, MAX_OUTPUT_LIMIT};
use crate::audit::Timer;
use crate::cli::{CommandText, Failure, Sink, warn, yes};
use crate::domain::{self, CancelSignal, ExecOutcome, Execution};
use crate::remote::exec as app_exec;
use crate::shell;
use crate::signals::Interrupt;
use crate::stdio::Stdout;
use crate::transport::{Client, StdinSource};
use crate::wire;
use std::io::{self, Write};
use std::time::Duration;

/// Runs one parsed `exec` against the host and delivers its answer. Returns the
/// process status: the remote program's own status when it completed, or the
/// mapped adapter status otherwise.
pub fn run(
    sink: &mut Sink,
    client: &Client,
    command: &Exec,
    json: bool,
    interrupt: &Interrupt,
) -> u8 {
    if command.timeout_nanos < 0
        || command
            .max_output_bytes
            .is_some_and(|limit| !(0..=MAX_OUTPUT_LIMIT).contains(&limit))
    {
        return Failure::new(
            "exec",
            &command.host,
            "CONFIG_INVALID",
            format!(
                "--timeout must be non-negative and --max-output-bytes between 0 and {MAX_OUTPUT_LIMIT}"
            ),
        )
        .deliver(sink, json);
    }
    let mut env: Vec<(String, String)> = Vec::new();
    for pair in &command.env {
        match pair.split_once('=') {
            Some((key, value)) if shell::is_valid_env_key(key) => {
                env.push((key.to_string(), value.to_string()));
            }
            _ => {
                return Failure::new(
                    "exec",
                    &command.host,
                    "CONFIG_INVALID",
                    format!("invalid --env {pair:?} (want a valid KEY=VALUE)"),
                )
                .deliver(sink, json);
            }
        }
    }
    let program = match read_command_file(command) {
        Ok(program) => program,
        Err(reason) => {
            return Failure::new("exec", &command.host, "CONFIG_INVALID", reason)
                .deliver(sink, json);
        }
    };
    let capture = match command.max_output_bytes {
        Some(limit) => Some(usize::try_from(limit).unwrap_or(usize::MAX)),
        None if json => Some(app_exec::DEFAULT_JSON_CAPTURE),
        None => None,
    };
    let timeout = if command.timeout_nanos > 0 {
        Some(Duration::from_nanos(command.timeout_nanos as u64))
    } else {
        // No default deadline: an ordinary foreground run waits for the remote
        // program to finish (EXEC-002).
        None
    };
    let request = app_exec::Request {
        host: &command.host,
        command: &program,
        cwd: command.cwd.as_deref(),
        env: &env,
        timeout,
        capture,
        fresh: command.fresh,
    };
    let (stdout_forward, stderr_forward) = if !json {
        (
            Some(Box::new(Stdout::open()) as Box<dyn Write>),
            Some(Box::new(io::stderr()) as Box<dyn Write>),
        )
    } else if command.stream {
        (
            Some(Box::new(io::stderr()) as Box<dyn Write>),
            Some(Box::new(io::stderr()) as Box<dyn Write>),
        )
    } else {
        (None, None)
    };
    let poll = || interrupt.requested();
    let reason = || interrupt.signal();
    // The trail records what was submitted, so it starts once the program and
    // its environment have been validated.
    let timer = Timer::start("exec", &command.host);
    let outcome = match app_exec::execute_stream(
        client,
        &request,
        stdout_forward,
        stderr_forward,
        StdinSource::Inherit,
        app_exec::Cancel::new(&poll, &reason),
    ) {
        Ok(outcome) => outcome,
        Err(reason) => {
            timer.failed("INTERNAL");
            return Failure::new("exec", &command.host, "INTERNAL", reason.to_string())
                .deliver(sink, json);
        }
    };
    match wire::exec_diagnostic(&outcome) {
        Some((code, _)) => timer.failed(code),
        None => {
            if let Execution::Completed(code) = outcome.execution() {
                timer.succeeded(
                    command.cwd.as_deref().unwrap_or(""),
                    &program,
                    Some(code.get()),
                );
            }
        }
    }
    let status = outcome.process_status();
    if json {
        sink.envelope(&wire::exec(&command.host, &outcome));
        return status;
    }
    if let Some((code, message)) = wire::exec_diagnostic(&outcome) {
        warn(&format!("rhost: {code}: {message}"));
        warn(&exec_status_line(&outcome));
    }
    status
}

/// The human counterpart of `data.execution`, `data.cleanup` and
/// `data.execution.cancel_signal`. Nothing here is a fact the envelope does not
/// also carry.
pub(crate) fn exec_status_line(outcome: &ExecOutcome) -> String {
    let execution = match outcome.execution() {
        Execution::Completed(code) => format!("execution=completed exit_code={}", code.get()),
        Execution::Unknown => "execution=unknown".to_string(),
        Execution::NotStarted => "execution=not_started".to_string(),
    };
    let failure = outcome.failure();
    let timed_out = matches!(
        failure,
        Some(domain::ExecFailure::Interrupted(
            domain::Interruption::Timeout
        ))
    );
    let signal = match failure {
        Some(domain::ExecFailure::Interrupted(domain::Interruption::Cancelled(signal))) => {
            Some(signal)
        }
        _ => None,
    };
    let mut line = format!(
        "{execution} timed_out={} cancelled={} cleanup_confirmed={}",
        yes(timed_out),
        yes(signal.is_some()),
        yes(outcome.cleanup() == domain::CleanupEvidence::ConfirmedStopped),
    );
    if let Some(signal) = signal {
        line.push_str(&format!(
            " cancel_signal={}",
            match signal {
                CancelSignal::Int => "SIGINT",
                CancelSignal::Term => "SIGTERM",
            }
        ));
    }
    line
}

/// Reads a `--command-file` operand locally. A symlink, a directory, a FIFO or an
/// empty file is refused before anything is sent, because "the program was
/// ambiguous" is not a question to answer by running it remotely.
pub(crate) fn read_command_file(command: &Exec) -> Result<String, String> {
    let CommandText::File(path) = &command.program else {
        return Ok(match &command.program {
            CommandText::Inline(text) => text.clone(),
            CommandText::File(_) => unreachable!("handled below"),
        });
    };
    let info =
        std::fs::symlink_metadata(path).map_err(|error| format!("read --command-file: {error}"))?;
    if !info.file_type().is_file() {
        return Err("--command-file must be a regular file".to_string());
    }
    if info.len() > MAX_COMMAND_FILE_BYTES {
        return Err(format!(
            "--command-file exceeds {MAX_COMMAND_FILE_BYTES} bytes"
        ));
    }
    let body = std::fs::read(path).map_err(|error| format!("read --command-file: {error}"))?;
    if body.is_empty() {
        return Err("--command-file must not be empty".to_string());
    }
    if body.len() as u64 > MAX_COMMAND_FILE_BYTES {
        return Err(format!(
            "--command-file exceeds {MAX_COMMAND_FILE_BYTES} bytes"
        ));
    }
    if body.contains(&0) {
        return Err("--command-file contains NUL".to_string());
    }
    String::from_utf8(body).map_err(|_| "--command-file is not valid UTF-8".to_string())
}
