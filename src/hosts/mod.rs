//! `hosts`: the concrete aliases this machine's OpenSSH client config names.
//!
//! Discovery is a best-effort read of the local client config; plain `ssh`
//! remains the authority on resolution, and the envelope says how complete the
//! read was so an agent never mistakes a partial list for the whole set.

use crate::cli::{PLAIN_FLAGS, ParsedCommand, Scope, Sink, parse, warn};
use crate::host;
mod dto;

use dto::hosts as hosts_dto;

/// `hosts` takes no operands and no flags beyond `--json`/`--help`, so the
/// parsed request carries nothing.
pub struct Hosts;

pub fn command(argv: &[String], json: bool) -> ParsedCommand<Hosts> {
    let parsed = match parse(argv, PLAIN_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Root, message),
    };
    if parsed.bool_flag("help") {
        return ParsedCommand::help_or_error(Scope::Root, crate::cli::Help::Hosts, json);
    }
    if !parsed.operands.is_empty() || parsed.dash {
        return ParsedCommand::usage(
            Scope::Root,
            format!(
                "rhost hosts accepts 0 arg(s), received {}",
                parsed.operands.len()
            ),
        );
    }
    ParsedCommand::Run(Hosts)
}

pub fn run(sink: &mut Sink, json: bool) -> u8 {
    let discovery = host::aliases();
    if json {
        sink.envelope(&hosts_dto(&discovery));
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
