//! Running `doctor`: probe the host and deliver what the probe proved.

use super::{Doctor, app, dto};
use crate::audit::Timer;
use crate::cli::{Failure, Sink, field, ok_missing, warn, yes};
use crate::transport::Client;
use crate::wire;
use std::time::Duration;

pub fn run(sink: &mut Sink, client: &Client, command: &Doctor, json: bool) -> u8 {
    let timer = Timer::start("doctor", &command.host);
    if command.timeout_nanos < 0 {
        timer.failed("CONFIG_INVALID");
        return Failure::new(
            "doctor",
            &command.host,
            "CONFIG_INVALID",
            "--timeout must be non-negative",
        )
        .deliver(sink, json);
    }
    let timeout = if command.timeout_nanos == 0 {
        Duration::from_nanos(super::DEFAULT_TIMEOUT_NANOS as u64)
    } else {
        Duration::from_nanos(command.timeout_nanos as u64)
    };
    let options = app::Options {
        host: &command.host,
        timeout,
        fresh: command.fresh,
    };
    let probe = match app::probe(client, &options) {
        Ok(probe) => probe,
        Err(reason) => {
            timer.failed("INTERNAL");
            return Failure::new("doctor", &command.host, "INTERNAL", reason.to_string())
                .deliver(sink, json);
        }
    };
    if let Some(outcome) = &probe.failure {
        let status = outcome.process_status();
        match wire::exec_diagnostic(outcome) {
            Some((code, _)) => timer.failed(code),
            // Every failure has a code, so this is an exhaustiveness guard rather
            // than a path: the honest answer when a failure had none is that the
            // execution is unknown.
            None => timer.failed("REMOTE_EXECUTION_UNKNOWN"),
        }
        if json {
            sink.envelope(&dto::doctor_failure(&probe.report, outcome));
        } else if let Some((code, message)) = wire::exec_diagnostic(outcome) {
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
        sink.envelope(&dto::doctor(&probe.report));
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
    for name in app::CAPABILITIES {
        field(sink, name, &capability_line(report, name));
    }
    0
}

/// One capability line: `OK (<resolved-path>)` when this execution environment
/// found the tool, `missing` otherwise. The path is the same value the envelope
/// carries under `data.capability_paths`, so a human and an agent read one fact.
fn capability_line(report: &app::Report, name: &str) -> String {
    match report.capability_paths.get(name).and_then(Option::as_deref) {
        Some(path) => format!("OK ({path})"),
        None => ok_missing(false).to_string(),
    }
}
