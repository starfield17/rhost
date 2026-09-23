//! Running one tunnel operation and mapping a fault onto the error taxonomy.

use super::command::Command as TunnelCommand;
use super::{Fault, dto, render};
use crate::audit::Timer;
use crate::cli::{Failure, Sink, warn};
use crate::transport::Client;

/// Runs one tunnel operation, delivering the envelope or the human view, and
/// mapping a fault exactly once so a code never has to be repeated.
pub fn run(sink: &mut Sink, client: &Client, command: TunnelCommand, json: bool) -> u8 {
    match command {
        TunnelCommand::Open {
            host,
            kind,
            listen,
            destination,
            expose,
        } => {
            let timer = Timer::start("tunnel.open", &host);
            let request = match super::Request::new(&host, &kind, &listen, &destination, expose) {
                Ok(request) => request,
                Err(fault) => {
                    let failure = fault_failure("tunnel.open", &host, fault);
                    timer.failed(failure.code());
                    return failure.deliver(sink, json);
                }
            };
            match super::open(client, &request) {
                Ok(opened) => {
                    timer.succeeded("", &format!("{kind} {listen} -> {destination}"), None);
                    if json {
                        sink.envelope(&dto::tunnel("tunnel.open", &host, &opened));
                    } else {
                        render::tunnel(sink, &opened);
                    }
                    0
                }
                Err(fault) => {
                    let failure = fault_failure("tunnel.open", &host, fault);
                    timer.failed(failure.code());
                    failure.deliver(sink, json)
                }
            }
        }
        TunnelCommand::List => match super::list(client) {
            Ok(rows) => {
                if json {
                    sink.envelope(&dto::tunnels(&rows));
                } else {
                    render::tunnels(sink, &rows);
                }
                0
            }
            Err(fault) => fault_failure("tunnel.list", "", fault).deliver(sink, json),
        },
        TunnelCommand::Close { id } => {
            let timer = Timer::start("tunnel.close", "");
            match super::close(client, &id) {
                Ok(closed) => {
                    timer.succeeded("", &closed, None);
                    if json {
                        sink.envelope(&dto::tunnel_closed(&closed));
                    } else {
                        warn(&format!("closed {closed}"));
                    }
                    0
                }
                Err(fault) => {
                    let failure = fault_failure("tunnel.close", "", fault);
                    timer.failed(failure.code());
                    failure.deliver(sink, json)
                }
            }
        }
    }
}

/// A missing record is not the same problem as a refused forward: an agent that
/// wants to clean up has to be able to tell "already gone" from "could not be
/// stopped", and only the transport-shaped failure is worth retrying.
fn fault_failure(operation: &'static str, host: &str, fault: Fault) -> Failure {
    match fault {
        Fault::Invalid(message) => Failure::new(operation, host, "CONFIG_INVALID", message),
        Fault::NotFound(message) => Failure::new(operation, host, "TUNNEL_NOT_FOUND", message),
        Fault::Transport(message) => {
            Failure::new(operation, host, "TUNNEL_FAILED", message).retryable()
        }
        Fault::Uncertain(message) => Failure::new(operation, host, "TUNNEL_FAILED", message),
    }
}
