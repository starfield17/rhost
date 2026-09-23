//! The `tunnel` command surface: grammar, running and the mapping of a tunnel
//! fault onto the error taxonomy.
//!
//! The three subcommands are thin on purpose. Every decision about what a forward
//! may bind, and whether one is alive, belongs to `crate::tunnel` and to OpenSSH;
//! what this adds is the agent-facing contract — an id to keep, a status to
//! branch on, and one document per call (AGENTS.md §6).
use crate::cli::{FlagSpec, Help, JSON, PLAIN_FLAGS, ParsedCommand, Scope, parse};

/// Every tunnel operation the parser can hand to [`run`].
pub enum Command {
    Open {
        host: String,
        kind: String,
        listen: String,
        destination: String,
        expose: bool,
    },
    List,
    Close {
        id: String,
    },
}

/// The flags `open` takes. The group accepts them so the leaf can be found, and
/// every other leaf then parses the same argv again with its own table, which is
/// what refuses a flag that belongs to a sibling operation.
static TUNNEL_FLAGS: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("kind", false, "forward kind (local, reverse or socks)"),
    FlagSpec::value("listen", false, "address to listen on"),
    FlagSpec::value("destination", false, "forward destination host:port"),
    FlagSpec::long("allow-exposure", "permit binding a non-loopback address"),
];

/// The flags each leaf accepts, so its `--help` page and its parse read the same
/// table. `list` and `close` take operands only.
pub fn leaf_flags(leaf: &str) -> &'static [FlagSpec] {
    if leaf == "open" {
        TUNNEL_FLAGS
    } else {
        PLAIN_FLAGS
    }
}

/// Defaults that match the reference: the common case is a local forward out of
/// loopback, and a caller who wants something else says so.
const DEFAULT_KIND: &str = "local";
const DEFAULT_LISTEN: &str = "localhost:8080";

pub fn command(argv: &[String], json: bool) -> ParsedCommand<Command> {
    let scanned = match parse(argv, TUNNEL_FLAGS) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Tunnel, message),
    };
    let leaf = scanned.operand(0).map(str::to_string);
    if scanned.bool_flag("help") {
        if json {
            return ParsedCommand::help_or_error(Scope::Tunnel, Help::Group(Scope::Tunnel), json);
        }
        return match &leaf {
            Some(leaf) => ParsedCommand::HelpLeaf(Scope::Tunnel, leaf.clone()),
            None => ParsedCommand::Help(Help::Group(Scope::Tunnel)),
        };
    }
    let Some(leaf) = leaf else {
        return ParsedCommand::usage(Scope::Tunnel, "rhost tunnel needs a subcommand".to_string());
    };
    let parsed = match parse(argv, leaf_flags(&leaf)) {
        Ok(parsed) => parsed,
        Err(message) => return ParsedCommand::usage(Scope::Tunnel, message),
    };
    let operands = &parsed.operands[1.min(parsed.operands.len())..];
    match leaf.as_str() {
        "open" => {
            if let Err(message) = exact(&leaf, operands, 1, "<host>") {
                return ParsedCommand::usage(Scope::Tunnel, message);
            }
            let flag =
                |name: &str, default: &str| parsed.value(name).unwrap_or(default).to_string();
            ParsedCommand::Run(Command::Open {
                host: operands[0].clone(),
                kind: flag("kind", DEFAULT_KIND),
                listen: flag("listen", DEFAULT_LISTEN),
                destination: flag("destination", ""),
                expose: parsed.bool_flag("allow-exposure"),
            })
        }
        "list" => {
            if let Err(message) = exact(&leaf, operands, 0, "") {
                return ParsedCommand::usage(Scope::Tunnel, message);
            }
            ParsedCommand::Run(Command::List)
        }
        "close" => {
            if let Err(message) = exact(&leaf, operands, 1, "<id>") {
                return ParsedCommand::usage(Scope::Tunnel, message);
            }
            ParsedCommand::Run(Command::Close {
                id: operands[0].clone(),
            })
        }
        other => ParsedCommand::usage(
            Scope::Tunnel,
            format!("unknown command {other} for \"rhost tunnel\""),
        ),
    }
}

fn exact(leaf: &str, operands: &[String], count: usize, names: &str) -> Result<(), String> {
    if operands.len() == count {
        return Ok(());
    }
    let wanted = if names.is_empty() {
        String::new()
    } else {
        format!(" {names}")
    };
    Err(format!(
        "rhost tunnel {leaf} accepts {count} arg(s){wanted}, received {}",
        operands.len()
    ))
}
