//! The prose a human reads: the root's usage and each group's subcommand list.
//!
//! With `--json` none of this is printed; the envelope reports a usage error
//! instead, because prose on stdout would break a decoder (WIRE-002).

use super::Scope;

pub(crate) fn root() -> String {
    concat!(
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
        "\n",
        "Flags:\n",
        "  --json   emit exactly one machine-readable JSON document on stdout\n",
        "\n",
        "`audit` reads the local JSON Lines trail of remote operations; RHOST_AUDIT=0\n",
        "turns writing it off. `session attach` needs a terminal and is refused with\n",
        "USAGE_ERROR rather than pretending to attach.\n",
    )
    .to_string()
}

pub(crate) fn group(scope: Scope) -> String {
    let name = scope.name();
    let leaves: &[(&str, &str)] = match scope {
        Scope::Root => &[],
        Scope::Connection => &[
            ("status <host>", "Inspect the shared SSH master"),
            (
                "reset <host>",
                "Stop the shared SSH master accepting new requests",
            ),
        ],
        Scope::Session => &[
            ("create <host>", "Create a persistent tmux-backed session"),
            ("list <host>", "List sessions and their liveness"),
            (
                "exec <host> <session> --command <program>",
                "Run a command in a session (state persists)",
            ),
            (
                "send <host> <session> (--data TEXT [--enter] | --key KEY)",
                "Send raw input or a control key",
            ),
            (
                "read <host> <session> [--since N]",
                "Read a session's output log incrementally",
            ),
            (
                "recover <host> <session>",
                "Interrupt what is running and verify the shell answers",
            ),
            (
                "close <host> <session>",
                "Close a session and remove its state",
            ),
            (
                "attach <host> <session>",
                "needs a terminal; refused with USAGE_ERROR",
            ),
        ],
        Scope::Tunnel => &[
            ("open <host>", "Start one forward on its own OpenSSH master"),
            ("list", "List tunnels and check each master"),
            ("close <id>", "Close exactly one tunnel and its master"),
        ],
        Scope::Fs => &[
            (
                "put <host> <local> <remote>",
                "Copy one local file to the host",
            ),
            ("get <host> <remote> <local>", "Copy one remote file here"),
            (
                "sync <host> <local-dir> <remote-dir>",
                "Bring a remote tree in line",
            ),
            (
                "mirror <host> <remote-dir> <local-dir>",
                "Download a remote tree",
            ),
            (
                "batch <host> --manifest <file>",
                "Run an ordered list of copies",
            ),
            (
                "read <host> <path>",
                "Print a bounded slice of a remote file",
            ),
            (
                "write <host> <path>",
                "Create or hash-guarded replace a file",
            ),
            (
                "patch <host> <path> --patch <file>",
                "Apply a hash-guarded patch",
            ),
        ],
    };
    let mut text = format!("Usage: rhost {name} <subcommand>\n\nSubcommands:\n");
    for (leaf, short) in leaves {
        text.push_str(&format!("  {name} {leaf}\n    {short}\n"));
    }
    text
}
/// The group's own help text, or the root's.
pub(crate) fn help(scope: Scope) -> String {
    match scope {
        Scope::Root => root(),
        other => group(other),
    }
}
