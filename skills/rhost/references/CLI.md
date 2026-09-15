# rhost CLI reference

The installed binary's `--help` output is the authoritative command and flag
inventory: every command has its own page, and a flag that reaches a parser
without reaching its page fails the test suite. All structured responses use
schema v2 with top-level `schema_version`, `operation`, `ok`, `data`, and
`error`. Branch on `error.code`, never on English text; the full
`error.code` → recovery table lives in
[RECOVERY.md](RECOVERY.md). Confirm `rhost version --json` and
`rhost <command> --help` match this reference before relying on it, since a
symlinked skill and the installed binary can drift apart.

## Foreground execution

```bash
rhost exec <host> --command '<shell-program>'
rhost exec <host> --command-file ./inspect.sh
rhost exec <host> --cwd '<remote-directory>' --env KEY=value --command '<shell-program>'
rhost exec <host> --json --timeout 30s --command '<shell-program>'
rhost exec <host> --json --stream --command-file ./inspect.sh
rhost exec <host> --fresh --command '<shell-program>'
```

Supply exactly one non-empty program through `--command` (`-c`) or a regular,
non-empty `--command-file` of at most 64 KiB. The file is command text, not a
remote file or a secret container. Human mode forwards stdin and streams stdout
and stderr. JSON mode captures both streams separately; `--stream` additionally
mirrors live remote output to local stderr so stdout remains one JSON document.

There is no default exec deadline. JSON capture defaults to 1 MiB per stream;
`--max-output-bytes 0` removes the capture limit. A truncation flag means the
captured text is incomplete even though `bytes` reports all observed source
bytes.

The program runs under the target account's login shell, which resolves the
`ssh` wrapper and enters `bash -lc`; the environment that login already built is
the one the command sees. `--fresh` selects a new SSH connection, not a clean
`PATH`. rhost does not infer which package a tool belongs to: ask the host with
`command -V <tool>` and the host's own package manager, run through `exec`.

A `;`, pipe or redirection left *outside* the quoted `--command` value is
consumed by your local shell before rhost runs anything; rhost cannot detect it.
Keep the whole program inside the value, or use `--command-file`, which sends the
file's text as one program without creating any remote script.

For `exec`, read:

- `data.execution.status`: `completed`, `unknown`, or `not_started`;
- `data.execution.exit_code`: present only for completed execution;
- `data.output.kind == "streams"`, then `stdout` and `stderr` objects containing
  `content`, `bytes`, and `truncated`;
- `data.cleanup.status`: `not_attempted`, `confirmed_stopped`, or `unconfirmed`;
- `data.cancel_signal` when local SIGINT or SIGTERM ended the invocation.

A `REMOTE_COMMAND_TIMEOUT` is the *managed* command exceeding its deadline: the
wrapper's matched process group is the thing that may have been stopped, and the
command may already have had effects. `TRANSFER_FAILED` is different: it is a
local `scp`/`rsync` failure after a transfer was attempted, so the tool's own
diagnostic and the real destination are the evidence. Do not conflate them.

### Small scripts and exploration

`--command` accepts literal newlines. For simple quoting, a short set of
independent read-only probes can be one program:

```bash
rhost exec gpu --command '
uname -a
getconf _NPROCESSORS_ONLN
df -h
'
```

Each probe above runs even if an earlier one fails; the shell exit status is
that of the last command. Use explicit checks when later work depends on an
earlier result. Do not combine steps whose next command requires interpreting
the previous output.

When nesting quotes or adding branches becomes awkward, write an ordinary local
script such as `inspect.sh` and pass its path:

```bash
rhost exec gpu --json --stream --command-file ./inspect.sh > result.json
jq -r '.data.output.stdout.content' result.json
```

The file supplies program text while stdin remains available to the remote
program. `--command-file -` and process substitution are not supported: this
operand must be a regular local file. No persistent session is needed just to
run a script. JSON string escaping is decoded by `jq -r`; pretty-printing the
envelope alone does not turn captured text into multiple readable lines.

## Hosts and diagnosis

```bash
rhost hosts --json
rhost connection status <host> --json
rhost doctor <host> --fresh --json
rhost connection reset <host> --json
```

`hosts` reads client configuration without contacting a host. `doctor --timeout
0` uses its default 60-second probe budget. `connection reset` stops a shared
master accepting new channels; already accepted channels continue. `--fresh`
uses no shared ControlMaster state.

Default budgets, for callers who do not name a deadline:

- `doctor`: 60 s probe.
- transfers (`fs put`/`get`/`sync`/`mirror` and each `fs batch` item): 5 minutes
  per item — the batch budget is per entry, not for the whole list.
- file helpers (`fs read`/`write`/`patch`): 60 s.
- `session create`: 60 s; other session operations (`list`, `exec`, `send`,
  `read`, `recover`, `close`): 30 s, except `exec`, whose 60 s covers the command.

`doctor --timeout 0` and an omitted `--timeout` both select these defaults.

For a frequently used target, an optional entry in `~/.ssh/config` avoids
repeating the login and hostname:

```sshconfig
Host gpu
    HostName example-host
    User user
```

Then use `rhost exec gpu --command 'uname -a'`. `hosts` enumerates existing SSH
configuration aliases; an empty list does not prevent using `user@example-host`
directly. rhost does not maintain a separate alias registry.

Ordinary calls use OpenSSH `ControlMaster=auto` and `ControlPersist=15m`.
To investigate slow repeated calls, inspect `connection status` and run
`doctor <host> --json` to obtain reuse evidence. Reserve `doctor --fresh` for
testing an independent connection. Elapsed time alone cannot distinguish a
new handshake from remote login-shell startup or command execution costs.

Useful paths include `data.hosts[]`, `data.complete`, `data.warnings[]`,
`data.capabilities`, `data.capability_paths`, `data.state_dir`,
`data.connection.master_status`, `data.connection_reused`, `data.master_status`,
and `data.control_path`. `data.capability_paths` uses the same keys as
`data.capabilities` and gives the path this execution environment resolves each
tool to — the same `command -v` answer a real run would get — or `null` when the
tool is absent. A failed probe reports both maps empty.

## Sessions

```bash
rhost session create <host> --json --name debug --cwd '<remote-directory>'
rhost session list <host> --json
rhost session exec <host> debug --command '<shell-program>' --json
rhost session send <host> debug --data 'python3 -i' --enter
rhost session send <host> debug --key C-c
rhost session read <host> debug --json --since 0
rhost session recover <host> debug --json
rhost session close <host> debug --json
```

`session create` accepts `--name`, `--cwd`, `--shell bash`, and `--timeout`.
Exec, list, read, and recover also accept their documented timeout. Writers are
serialized remotely. `SESSION_BUSY` means another program owns the pane and the
requested command was not submitted. `send --data` is verbatim; `--enter` pastes
the data and presses Enter in the same submission.

Session exec returns `data.session_id`, `data.session_ref`, `data.execution`,
`data.session_preserved`, and PTY output at `data.output.content` with
`data.output.kind == "pty"`. Incremental reads use `data.from`, `data.next`, and
`data.more`. `session attach` returns a non-retryable `USAGE_ERROR`; the v2
envelope has no honest terminal attachment representation.

`session create --cwd` records the *absolute* directory the account actually
entered, so `~` comes back as a real path and `create` and a later `list` agree.
Without `--cwd`, `initial_cwd` is omitted, and the session starts wherever the
login shell leaves it.

A create that reaches the host but does not return its full record reports the
candidate it reserved under `data.session_id`/`data.session_ref` and a
`data.creation_status` of `not_created` (the remote refused, or was seen to clean
up), `unknown` (a timeout, cancellation, disconnect or missing evidence), or
`created` (creation was observed but the rest of the response was not). Use
`session list` to confirm a candidate instead of retrying blind. A local
argument mistake or an id that cannot be generated still reports `data: null`.

## Files

```bash
rhost fs put <host> ./local '<remote-path>' [--parents] [--resume] [--checksum]
rhost fs get <host> '<remote-path>' ./local [--resume] [--checksum]
rhost fs sync <host> ./directory '<remote-directory>' --dry-run --json
rhost fs mirror <host> '<remote-directory>' ./directory --dry-run --json
rhost fs read <host> '<remote-file>' --lines 200 --start 1 --json
rhost fs write <host> '<remote-file>' --from ./file --if-hash <sha256> --json
rhost fs patch <host> '<remote-file>' --patch ./patch.json --if-hash <sha256> --json
rhost fs batch <host> --manifest ./copies.json --json
```

Quote a leading `~` so the local shell does not expand it. Plain put/get use
scp; resume or checksum uses rsync. Sync/mirror accept repeated `--exclude`,
`--checksum`, `--delete`, `--dry-run`, and a deadline. Always preview the exact
destructive command with `--dry-run` before adding `--delete`.

`fs read` returns bounded UTF-8 text and the complete file's SHA-256. Creating a
new file needs no hash. Replacing an existing file and every patch require the
fresh hash returned by the last read. Batch is ordered serial put/get, continues
after individual failures, and reports `data.items[]`, `data.succeeded`, and
`data.failed`; it is not a transaction.

`fs read --max-bytes` defaults to 256 KiB and is capped at 8 MiB; a larger value
is refused as `CONFIG_INVALID`. `fs write --mode` is three or four octal digits;
when it is omitted a newly created file is `0600` and a replacement keeps the
existing file's permissions.

## Tunnels and audit

```bash
rhost tunnel open <host> --kind local --listen localhost:8080 --destination localhost:8000 --json
rhost tunnel list --json
rhost tunnel close <tunnel-id> --json
rhost audit --json --limit 20 --host <host>
```

Tunnels bind loopback unless `--allow-exposure` is explicit. Keep
`data.tunnel_id`; list entries use `data.tunnels[].tunnel_id`. `alive` means the
forward exists, not that an application answers behind it. Audit is local,
bounded, fail-open, and can be disabled with `RHOST_AUDIT=0`.
