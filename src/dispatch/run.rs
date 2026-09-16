//! Executing one parsed command: the thin match that routes to each capability.

use super::{Command, deliver_refused, deliver_usage};
use crate::cli::Sink;
use crate::signals::Interrupt;
use crate::transport::Client;
use crate::wire;

/// Routes one command to its capability and returns the process status. This is
/// the only place capabilities are wired together; each arm is one call.
pub(crate) fn dispatch(
    sink: &mut Sink,
    client: &Client,
    command: Command,
    json: bool,
    interrupt: &Interrupt,
) -> u8 {
    match command {
        Command::Version => {
            if json {
                sink.envelope(&wire::version());
            } else {
                version(sink);
            }
            0
        }
        Command::Help(help) => {
            sink.text(&super::usage::help(help));
            0
        }
        Command::HelpLeaf(scope, leaf) => {
            sink.text(&super::usage::help(super::usage::leaf_or_group(
                scope, &leaf,
            )));
            0
        }
        Command::Hosts(_request) => crate::hosts::run(sink, json),
        Command::Doctor(request) => crate::doctor::run(sink, client, &request, json),
        Command::Connection(request) => crate::connection::run(sink, client, request, json),
        Command::Exec(request) => crate::exec::run(sink, client, &request, json, interrupt),
        Command::Fs(request) => {
            // One interruption source for every operation that starts a process,
            // so a signal the CLI can receive is a signal it can act on.
            let stop_requested = || interrupt.requested();
            let stop_signal = || interrupt.signal();
            let stop = crate::files::Stop {
                requested: &stop_requested,
                signal: &stop_signal,
            };
            crate::files::run(sink, client, request, json, stop)
        }
        Command::Tunnel(request) => crate::tunnel::run(sink, client, request, json),
        Command::Session(request) => crate::session::run(sink, client, request, json),
        Command::Audit(request) => crate::audit::run(sink, request.limit, &request.host, json),
        Command::Refused { operation, message } => deliver_refused(sink, operation, &message, json),
        Command::Usage(rejection) => deliver_usage(sink, rejection.scope, &rejection.message, json),
    }
}

fn version(sink: &mut Sink) {
    use crate::cli::field;
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
