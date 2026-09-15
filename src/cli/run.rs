//! Running the non-file commands and printing their human view.
//!
//! Everything here goes through the application layer: the CLI decides what the
//! caller asked for, and `app` decides what it means.

use super::audit::Timer;
use super::console::{Failure, Sink, field, ok_missing, warn, yes};
use super::{CommandText, Exec, MAX_COMMAND_FILE_BYTES, MAX_OUTPUT_LIMIT};
use crate::app::doctor as app_doctor;
use crate::app::exec as app_exec;
use crate::domain::{self, CancelSignal, ExecOutcome, Execution};
use crate::host;
use crate::output;
use crate::shell;
use crate::signals::Interrupt;
use crate::stdio::Stdout;
use crate::transport::openssh::Client;
use crate::transport::process::StdinSource;
use std::io::{self, Write};
use std::time::Duration;

pub(crate) fn version(sink: &mut Sink) {
    sink.line(&format!("rhost {}", env!("CARGO_PKG_VERSION")));
    field(
        sink,
        "commit",
        option_env!("RHOST_BUILD_COMMIT").unwrap_or("none"),
    );
    field(
        sink,
        "build date",
        option_env!("RHOST_BUILD_DATE").unwrap_or("unknown"),
    );
    field(sink, "schema version", "2");
}

pub(crate) fn hosts(sink: &mut Sink, json: bool) -> u8 {
    let discovery = host::aliases();
    if json {
        sink.envelope(&output::hosts(&discovery));
        return 0;
    }
    if discovery.hosts.is_empty() {
        warn("no host aliases found in the OpenSSH client config");
    }
    for info in &discovery.hosts {
        sink.line(&info.alias);
    }
    for warning in &discovery.warnings {
        warn(warning);
    }
    if !discovery.complete {
        warn("host discovery is incomplete: plain ssh remains the authority on resolution");
    }
    0
}

pub(crate) fn connection_status(sink: &mut Sink, client: &Client, host: &str, json: bool) -> u8 {
    let status = client.connection_status(host);
    if json {
        let data = output::ConnectionDto::from(&status);
        sink.envelope(&output::connection("connection.status", host, &data));
        return 0;
    }
    field(sink, "Master status", status.master_status.as_str());
    field(sink, "Control path", &status.control_path);
    if let Some(pid) = status.master_pid {
        field(sink, "Master PID", &pid.to_string());
    }
    if !status.diagnostic.is_empty() {
        field(sink, "Diagnostic", &status.diagnostic);
    }
    0
}

pub(crate) fn connection_reset(sink: &mut Sink, client: &Client, host: &str, json: bool) -> u8 {
    let timer = Timer::start("connection.reset", host);
    let reset = client.reset_connection(host);
    if let Some(reason) = &reset.error {
        timer.failed("SSH_CONTROL_FAILED");
        let message = format!("could not stop the shared SSH master: {reason}");
        if json {
            let data = output::ConnectionDto::from(&reset.status);
            sink.envelope(&output::connection_failure(host, &data, &message));
        } else {
            warn(&format!("rhost: SSH_CONTROL_FAILED: {message}"));
        }
        return 255;
    }
    timer.succeeded("", "stop shared OpenSSH master", None);
    if json {
        let data = output::ConnectionDto::from(&reset.status);
        sink.envelope(&output::connection("connection.reset", host, &data));
        return 0;
    }
    sink.line(if reset.status.stopped {
        "shared SSH master stopped accepting new requests"
    } else {
        "no shared SSH master"
    });
    0
}

pub(crate) fn doctor(
    sink: &mut Sink,
    client: &Client,
    host: &str,
    timeout_nanos: i64,
    fresh: bool,
    json: bool,
) -> u8 {
    let timer = Timer::start("doctor", host);
    if timeout_nanos < 0 {
        timer.failed("CONFIG_INVALID");
        return Failure::new(
            "doctor",
            host,
            "CONFIG_INVALID",
            "--timeout must be non-negative",
        )
        .deliver(sink, json);
    }
    let timeout = if timeout_nanos == 0 {
        Duration::from_nanos(super::commands::DOCTOR_DEFAULT_TIMEOUT_NANOS as u64)
    } else {
        Duration::from_nanos(timeout_nanos as u64)
    };
    let options = app_doctor::Options {
        host,
        timeout,
        fresh,
    };
    let probe = match app_doctor::probe(client, &options) {
        Ok(probe) => probe,
        Err(reason) => {
            timer.failed("INTERNAL");
            return Failure::new("doctor", host, "INTERNAL", reason.to_string())
                .deliver(sink, json);
        }
    };
    if let Some(outcome) = &probe.failure {
        let status = outcome.process_status();
        match output::exec_diagnostic(outcome) {
            Some((code, _)) => timer.failed(code),
            // Every failure has a code, so this is an exhaustiveness guard rather
            // than a path: the honest answer when a failure had none is that the
            // execution is unknown.
            None => timer.failed("REMOTE_EXECUTION_UNKNOWN"),
        }
        if json {
            sink.envelope(&output::doctor_failure(&probe.report, outcome));
        } else if let Some((code, message)) = output::exec_diagnostic(outcome) {
            warn(&format!("rhost: {code}: {message}"));
            warn(&format!(
                "master_status={} control_path={}",
                probe.report.connection.master_status.as_str(),
                probe.report.connection.control_path
            ));
        }
        return status;
    }
    timer.succeeded("", "", None);
    if json {
        sink.envelope(&output::doctor(&probe.report));
        return 0;
    }
    let report = &probe.report;
    sink.line(&report.host);
    field(sink, "SSH", "OK (batch auth)");
    field(
        sink,
        "Shared master",
        report.connection.master_status.as_str(),
    );
    if let Some(reused) = report.connection_reused {
        field(sink, "Connection reused", yes(reused));
    }
    field(sink, "OS", &report.os);
    field(sink, "Kernel", &report.kernel);
    field(sink, "Arch", &report.arch);
    field(sink, "User", &report.user);
    field(sink, "Login shell", &report.login_shell);
    field(
        sink,
        "State dir",
        &format!(
            "{}  writable={}",
            report.state_dir,
            yes(report.state_dir_writable)
        ),
    );
    field(sink, "WSL", yes(report.wsl));
    sink.line("");
    for name in app_doctor::CAPABILITIES {
        field(sink, name, &capability_line(report, name));
    }
    0
}

pub(crate) fn exec(
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
    match output::exec_diagnostic(&outcome) {
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
        sink.envelope(&output::exec(&command.host, &outcome));
        return status;
    }
    if let Some((code, message)) = output::exec_diagnostic(&outcome) {
        warn(&format!("rhost: {code}: {message}"));
        warn(&exec_status_line(&outcome));
    }
    status
}

/// One capability line: `OK (<resolved-path>)` when this execution environment
/// found the tool, `missing` otherwise. The path is the same value the envelope
/// carries under `data.capability_paths`, so a human and an agent read one fact.
fn capability_line(report: &app_doctor::Report, name: &str) -> String {
    match report.capability_paths.get(name).and_then(Option::as_deref) {
        Some(path) => format!("OK ({path})"),
        None => ok_missing(false).to_string(),
    }
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
