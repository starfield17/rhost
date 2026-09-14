# rhost CLI reference

The installed binary's `--help` output is the authoritative command and flag
inventory: every command has its own page, and a flag that reaches a parser
without reaching its page fails the test suite. All structured responses use
schema v2 with top-level `schema_version`, `operation`, `ok`, `data`, and
`error`.

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

For `exec`, read:

- `data.execution.status`: `completed`, `unknown`, or `not_started`;
- `data.execution.exit_code`: present only for completed execution;
- `data.output.kind == "streams"`, then `stdout` and `stderr` objects containing
  `content`, `bytes`, and `truncated`;
- `data.cleanup.status`: `not_attempted`, `confirmed_stopped`, or `unconfirmed`;
- `data.cancel_signal` when local SIGINT or SIGTERM ended the invocation.

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
`data.capabilities`, `data.state_dir`, `data.connection.master_status`,
`data.connection_reused`, `data.master_status`, and `data.control_path`.

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
