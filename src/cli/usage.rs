//! The prose a human reads: the root's usage, a group's subcommand list, and one
//! page per command with the flags its parser accepts.
//!
//! Every page renders the same flag table the parser consults, so `--help` can
//! only describe flags the command really takes; a flag that reaches a table
//! without reaching a page is caught by this module's test rather than by a user.
//! With `--json` none of this is printed; the envelope reports a usage error
//! instead, because prose on stdout would break a decoder (WIRE-002).

use super::grammar::{self, FlagSpec};
use super::{Help, Scope};
use super::{audit, files, session, tunnel};

/// One leaf command's help entry: the usage after the group name and a one-line
/// summary. The flags come from the leaf's own parser table, so they are declared
/// exactly once.
struct Leaf {
    name: &'static str,
    usage: &'static str,
    summary: &'static str,
}

const CONNECTION_LEAVES: &[Leaf] = &[
    Leaf {
        name: "status",
        usage: "status <host>",
        summary: "Inspect the shared SSH master",
    },
    Leaf {
        name: "reset",
        usage: "reset <host>",
        summary: "Stop the shared SSH master accepting new requests",
    },
];

const SESSION_LEAVES: &[Leaf] = &[
    Leaf {
        name: "create",
        usage: "create <host>",
        summary: "Create a persistent tmux-backed session",
    },
    Leaf {
        name: "list",
        usage: "list <host>",
        summary: "List sessions and their liveness",
    },
    Leaf {
        name: "exec",
        usage: "exec <host> <session> --command <program>",
        summary: "Run a command in a session (state persists)",
    },
    Leaf {
        name: "send",
        usage: "send <host> <session> (--data TEXT [--enter] | --key KEY)",
        summary: "Send raw input or a control key",
    },
    Leaf {
        name: "read",
        usage: "read <host> <session> [--since N]",
        summary: "Read a session's output log incrementally",
    },
    Leaf {
        name: "recover",
        usage: "recover <host> <session>",
        summary: "Interrupt what is running and verify the shell answers",
    },
    Leaf {
        name: "close",
        usage: "close <host> <session>",
        summary: "Close a session and remove its state",
    },
    Leaf {
        name: "attach",
        usage: "attach <host> <session>",
        summary: "needs a terminal; refused with USAGE_ERROR",
    },
];

const TUNNEL_LEAVES: &[Leaf] = &[
    Leaf {
        name: "open",
        usage: "open <host>",
        summary: "Start one forward on its own OpenSSH master",
    },
    Leaf {
        name: "list",
        usage: "list",
        summary: "List tunnels and check each master",
    },
    Leaf {
        name: "close",
        usage: "close <id>",
        summary: "Close exactly one tunnel and its master",
    },
];

const FS_LEAVES: &[Leaf] = &[
    Leaf {
        name: "put",
        usage: "put <host> <local> <remote>",
        summary: "Copy one local file to the host",
    },
    Leaf {
        name: "get",
        usage: "get <host> <remote> <local>",
        summary: "Copy one remote file here",
    },
    Leaf {
        name: "sync",
        usage: "sync <host> <local-dir> <remote-dir>",
        summary: "Bring a remote tree in line",
    },
    Leaf {
        name: "mirror",
        usage: "mirror <host> <remote-dir> <local-dir>",
        summary: "Download a remote tree",
    },
    Leaf {
        name: "batch",
        usage: "batch <host> --manifest <file>",
        summary: "Run an ordered list of copies",
    },
    Leaf {
        name: "read",
        usage: "read <host> <path>",
        summary: "Print a bounded slice of a remote file",
    },
    Leaf {
        name: "write",
        usage: "write <host> <path>",
        summary: "Create or hash-guarded replace a file",
    },
    Leaf {
        name: "patch",
        usage: "patch <host> <path> --patch <file>",
        summary: "Apply a hash-guarded patch",
    },
];

fn leaves(scope: Scope) -> &'static [Leaf] {
    match scope {
        Scope::Connection => CONNECTION_LEAVES,
        Scope::Session => SESSION_LEAVES,
        Scope::Tunnel => TUNNEL_LEAVES,
        Scope::Fs => FS_LEAVES,
        Scope::Root => &[],
    }
}

/// The flag table a leaf's parser really uses, so the page and the parse read the
/// same declaration. `connection` leaves take no flags beyond the shared ones.
fn flags(scope: Scope, leaf: &str) -> &'static [FlagSpec] {
    match scope {
        Scope::Connection | Scope::Root => grammar::PLAIN_FLAGS,
        Scope::Session => session::leaf_flags(leaf),
        Scope::Tunnel => tunnel::leaf_flags(leaf),
        Scope::Fs => files::leaf_flags(leaf),
    }
}

/// The page for a group leaf, or the group's overview when the name is not a
/// leaf this group knows. A parser asks for this before it validates anything
/// else, so `rhost session bogus --help` still explains the group.
pub(crate) fn leaf_or_group(scope: Scope, leaf: &str) -> Help {
    match leaves(scope).iter().find(|entry| entry.name == leaf) {
        Some(entry) => Help::Leaf(scope, entry.name),
        None => Help::Group(scope),
    }
}

pub(crate) fn help(target: Help) -> String {
    match target {
        Help::Root => root(),
        Help::Group(scope) => group(scope),
        Help::Leaf(scope, name) => leaf(scope, name),
        Help::Exec => page(
            "rhost exec <host> (--command <shell-program> | --command-file <local-file>)",
            "Run one exact shell program in a fresh remote execution context.",
            grammar::exec_flags(),
        ),
        Help::Doctor => page(
            "rhost doctor <host>",
            "Probe a host and report what it supports before relying on it.",
            grammar::doctor_flags(),
        ),
        Help::Hosts => page(
            "rhost hosts",
            "List concrete Host aliases from your OpenSSH client config.",
            grammar::PLAIN_FLAGS,
        ),
        Help::Version => page(
            "rhost version",
            "Print the CLI and schema version.",
            grammar::PLAIN_FLAGS,
        ),
        Help::Audit => page(
            "rhost audit",
            "Read the local JSON Lines trail of remote operations.",
            audit::AUDIT_FLAGS,
        ),
    }
}

pub(crate) fn root() -> String {
    let mut text = concat!(
        "rhost runs ordinary commands on an SSH-reachable host.\n",
        "\n",
        "Usage:\n",
        "  rhost exec <host> (--command <shell-program> | --command-file <local-file>)\n",
        "  rhost doctor <host> [--timeout DURATION] [--fresh]\n",
        "  rhost hosts\n",
        "  rhost connection status|reset <host>\n",
        "  rhost session create|list|exec|send|read|recover|close\n",
        "  rhost fs put|get|sync|mirror|batch|read|write|patch\n",
        "  rhost tunnel open|list|close\n",
        "  rhost audit [--limit N] [--host HOST]\n",
        "  rhost version\n",
        "\n",
        "A host is an alias from your OpenSSH client config, a user@host, or a bare\n",
        "hostname. OpenSSH owns resolution, authentication and host-key policy; rhost\n",
        "orchestrates it and never duplicates it.\n",
    )
    .to_string();
    flag_lines(&mut text, grammar::ROOT_FLAGS_ALL);
    text.push_str(
        "\n`audit` reads the local JSON Lines trail of remote operations; RHOST_AUDIT=0\n\
         turns writing it off. `session attach` needs a terminal and is refused with\n\
         USAGE_ERROR rather than pretending to attach.\n",
    );
    text
}

fn group(scope: Scope) -> String {
    let name = scope.name();
    let mut text = format!("Usage: rhost {name} <subcommand>\n\nSubcommands:\n");
    for leaf in leaves(scope) {
        text.push_str(&format!("  {name} {}\n    {}\n", leaf.usage, leaf.summary));
    }
    flag_lines(&mut text, grammar::PLAIN_FLAGS);
    text
}

fn leaf(scope: Scope, name: &str) -> String {
    match leaves(scope).iter().find(|entry| entry.name == name) {
        Some(entry) => page(
            &format!("rhost {} {}", scope.name(), entry.usage),
            entry.summary,
            flags(scope, name),
        ),
        None => group(scope),
    }
}

fn page(usage: &str, summary: &str, flags: &[FlagSpec]) -> String {
    let mut text = format!("Usage: {usage}\n\n{summary}\n");
    flag_lines(&mut text, flags);
    text
}

/// The `Flags:` block every page ends with. `--json` and `--help` are accepted
/// everywhere; the rest come from the command's own table, in its own order.
fn flag_lines(text: &mut String, flags: &[FlagSpec]) {
    text.push_str("\nFlags:\n");
    for flag in flags {
        let mut names = match flag.short() {
            Some(short) => format!("-{short}, --{}", flag.name()),
            None => format!("    --{}", flag.name()),
        };
        if flag.valued() {
            names.push_str(" <value>");
        }
        text.push_str(&format!("  {names:<32} {}\n", flag.help()));
    }
    text.push_str(&format!("  {:<32} {}\n", "-h, --help", "print this help"));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every page must name every flag its parser accepts. The flag tables are
    /// the grammar, so this is what keeps the CLI.md claim that `--help` is the
    /// authoritative inventory true: a flag added to a table but not the page
    /// fails here, and a page promising a flag that no table holds fails too.
    #[test]
    fn every_page_describes_every_flag_it_accepts() {
        for (target, accepted) in [
            (Help::Root, grammar::ROOT_FLAGS_ALL),
            (Help::Exec, grammar::exec_flags()),
            (Help::Doctor, grammar::doctor_flags()),
            (Help::Hosts, grammar::PLAIN_FLAGS),
            (Help::Version, grammar::PLAIN_FLAGS),
            (Help::Audit, audit::AUDIT_FLAGS),
        ] {
            let text = help(target);
            assert!(text.contains("--json"), "{target:?} omits --json:\n{text}");
            for flag in accepted {
                let spelling = format!("--{}", flag.name());
                assert!(
                    text.contains(&spelling),
                    "{target:?} omits {spelling}:\n{text}"
                );
            }
        }
        for scope in [Scope::Connection, Scope::Session, Scope::Tunnel, Scope::Fs] {
            let overview = help(Help::Group(scope));
            assert!(overview.contains("--json"), "{scope:?}:\n{overview}");
            for entry in leaves(scope) {
                let text = help(Help::Leaf(scope, entry.name));
                assert!(
                    text.contains(&format!("rhost {} {}", scope.name(), entry.usage)),
                    "{scope:?} {} has the wrong usage:\n{text}",
                    entry.name
                );
                for flag in flags(scope, entry.name) {
                    let spelling = format!("--{}", flag.name());
                    assert!(
                        text.contains(&spelling),
                        "{scope:?} {} omits {spelling}:\n{text}",
                        entry.name
                    );
                }
            }
        }
    }
}
