//! The only entry point: install interruption handling, parse, run, deliver.
//!
//! Nothing durable is owned here. Connections belong to OpenSSH's control
//! socket and sessions to remote tmux, so this process may exit at any moment
//! without taking state with it (AGENTS.md §4).
#![forbid(unsafe_code)]
use rhost::cli;
use rhost::signals::Interrupt;
use rhost::transport::{Client, Config};
use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    // Handlers go up before anything can block: a CLI that starts a remote
    // command it cannot be asked to stop is not one an agent can trust.
    let interrupt = match Interrupt::install() {
        Ok(interrupt) => interrupt,
        Err(error) => {
            let mut stderr = std::io::stderr().lock();
            let _ = writeln!(
                stderr,
                "rhost: INTERNAL: could not install signal handlers: {error}"
            );
            return ExitCode::from(255);
        }
    };
    let invocation = cli::parse_invocation(&argv);
    let client = Client::new(Config::default());
    let delivery = invocation.run(&client, &interrupt);
    ExitCode::from(if delivery.delivery_failed {
        255
    } else {
        delivery.status
    })
}
