# rhost CLI reference

The installed binary's `--help` output is the authoritative command and flag
inventory: every command has its own page, and a flag that reaches a parser
without reaching its page fails the test suite. All structured responses use
schema v3 with top-level `schema_version`, `operation`, `ok`, `data`, and
`error`. Branch on `error.code`, never on English text; the full
`error.code` → recovery table lives in
[RECOVERY.md](RECOVERY.md). After installation, upgrade, or a flag mismatch,
check `rhost version --json` and `rhost <command> --help`: a symlinked skill and
the installed binary can drift apart. Request help without `--json`;
`--help --json` is a `USAGE_ERROR`, not a structured command inventory.

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

The SSH command crosses the target account's configured shell and starts the
wrapper under `setsid bash -lc`. The program inherits that bash login startup
environment; it is not an interactive shell. `--fresh` selects a new SSH
connection, not a clean `PATH`. rhost does not infer which package a tool belongs to: ask the host with
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

### Privileged execution (`sudo`) and environment

Exec runs in a non-interactive login shell under `setsid bash -lc`.
Executing commands with `sudo` across rhost calls has specific constraints:

- Sudo timestamp caching (`timestamp_timeout`) does not survive across separate
  rhost calls when the remote sudo configuration uses `tty_tickets`. Each `rhost exec` is an independent non-tty session.
- `secure_path` in `/etc/sudoers` resets `PATH` to system directories
  (`/usr/bin:/bin:/usr/sbin:/sbin`), stripping user `PATH` additions like Homebrew
  (`/opt/homebrew/bin`), Conda/Mamba environments, or Cargo. Invoke tools by full path
  (e.g., `sudo /usr/bin/apt-get update`) or pass `sudo env PATH="$PATH" <tool>`.
- For multi-step privileged tasks, group them in a single script via `--command-file`,
  or execute them within an `rhost session`.
- If password input is required non-interactively, use `sudo -S` with stdin. Never
  put passwords into command text, arguments, or environment variables. For interactive
  password entry, use direct OpenSSH with a PTY: `ssh -t <host> 'sudo ...'`.

### Recoverable background work

Use native Runs for builds/tests and work that must survive CLI exit. Their
caller-owned expiring keys, separate output logs and durable completion remove
the need for manually managed PID files. `--wait` does not stop the process;
`--deadline` and `run signal` are explicit controls. See [RUNS.md](RUNS.md).
An external scheduler remains appropriate for scheduled/restarted services.

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

Ordinary calls use OpenSSH `ControlMaster=auto` and `ControlPersist=15m` (15-minute persistence).
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

## Direct OpenSSH on a minimal host

If a host lacks a dependency for the requested rhost operation, an agent with a
local command tool can use the system OpenSSH client. These commands run on the
local machine; they do not invoke `rhost exec` or its remote bash wrapper:

```bash
ssh user@example-host 'sh -c "uname -a"'
ssh user@example-host                    # run this in a local PTY for an interactive login
scp ./local-file user@example-host:/path/to/file
scp user@example-host:/path/to/file ./local-file
```

Prefer one-shot SSH commands when the next step does not need a live shell. For
interactive exploration, allocate a local PTY and keep the SSH process open;
the login uses the target account's shell, which may be `sh` or `ash`. If remote
PTY allocation must be forced, use `ssh -tt user@example-host`. The ordinary
remote shell state ends with that SSH connection; this is not a durable
`rhost session`. `rhost session attach` remains
unavailable. For file bytes, use `scp` only when the host supports SFTP or the
required scp service, and inspect both source and destination after a failed
transfer. A PTY is not a reliable byte-transfer channel.

Direct OpenSSH output is terminal text and exit status, not a schema-v3 JSON
envelope. Do not infer rhost completion or cleanup evidence from a shell prompt.
See [SAFETY.md](SAFETY.md) before replacing or deleting files this way.

## Sessions

For terminal lifecycle, REPL interaction, byte cursors and timeout semantics,
read [SESSIONS.md](SESSIONS.md). `session attach` remains refused.

## Files

Read [FILES.md](FILES.md) for native reads, CAS editing, search/read-many and
named changeset recovery. File copies and directory transfers remain CLI-only:

```bash
rhost fs put <host> ./local '<remote-path>' [--parents] [--resume] [--checksum]
rhost fs get <host> '<remote-path>' ./local [--resume] [--checksum]
rhost fs sync <host> ./directory '<remote-directory>' --dry-run --json
rhost fs mirror <host> '<remote-directory>' ./directory --dry-run --json
```

Quote a leading `~` so the local shell does not expand it. Plain put/get use
scp; resume or checksum uses rsync. Sync/mirror accept repeated `--exclude`,
`--checksum`, `--delete`, `--dry-run`, and a deadline. Exclude patterns are
source-relative. With `--delete`, sync prunes remotely and mirror prunes locally.
Preview with `--dry-run --delete`, then remove only `--dry-run` to apply.

### Batch manifest

Save this JSON array as `copies.json`. Local operands are resolved on the local
machine; remote operands belong to the host passed to `fs batch`:

```json
[
  {"op": "put", "source": "./upload.txt", "destination": "~/work/upload.txt"},
  {"op": "get", "source": "~/work/result.txt", "destination": "./download.txt"}
]
```

```bash
rhost fs batch gpu --manifest ./copies.json --json
```

Each entry accepts optional boolean `resume` and `checksum` fields, both false
by default. Parents must already exist. Entries run serially and later entries
still run after a failure. A completed batch report has `ok:true` even when an
entry failed: inspect `data.failed` and `data.items[].ok`/`error`; failed entries
make the process exit 255. Inspect destinations before replaying any entries.

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
