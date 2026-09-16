//! Running `connection status`/`reset` and delivering the master's state.

use super::Connection;
use super::dto::ConnectionDto;
use crate::audit::Timer;
use crate::cli::{Sink, field, warn};
use crate::transport::{Client, ConnectionStatus};

pub fn status(sink: &mut Sink, client: &Client, host: &str, json: bool) -> u8 {
    let status = client.connection_status(host);
    if json {
        let data = ConnectionDto::from(&status);
        sink.envelope(&super::connection("connection.status", host, &data));
        return 0;
    }
    print_status(sink, &status);
    0
}

pub fn reset(sink: &mut Sink, client: &Client, host: &str, json: bool) -> u8 {
    let timer = Timer::start("connection.reset", host);
    let reset = client.reset_connection(host);
    if let Some(reason) = &reset.error {
        timer.failed("SSH_CONTROL_FAILED");
        let message = format!("could not stop the shared SSH master: {reason}");
        if json {
            let data = ConnectionDto::from(&reset.status);
            sink.envelope(&super::connection_failure(host, &data, &message));
        } else {
            warn(&format!("rhost: SSH_CONTROL_FAILED: {message}"));
        }
        return 255;
    }
    timer.succeeded("", "stop shared OpenSSH master", None);
    if json {
        let data = ConnectionDto::from(&reset.status);
        sink.envelope(&super::connection("connection.reset", host, &data));
        return 0;
    }
    sink.line(if reset.status.stopped {
        "shared SSH master stopped accepting new requests"
    } else {
        "no shared SSH master"
    });
    0
}

fn print_status(sink: &mut Sink, status: &ConnectionStatus) {
    field(sink, "Master status", status.master_status.as_str());
    field(sink, "Control path", &status.control_path);
    if let Some(pid) = status.master_pid {
        field(sink, "Master PID", &pid.to_string());
    }
    if !status.diagnostic.is_empty() {
        field(sink, "Diagnostic", &status.diagnostic);
    }
}

/// Runs one parsed connection command.
pub fn run(sink: &mut Sink, client: &Client, command: Connection, json: bool) -> u8 {
    match command {
        Connection::Status { host } => status(sink, client, &host, json),
        Connection::Reset { host } => reset(sink, client, &host, json),
    }
}
