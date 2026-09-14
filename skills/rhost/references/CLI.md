# rhost CLI reference

The binary's `--help` output is the authoritative flag inventory. This document
explains behavior and keeps this compact coverage index so omissions fail tests:

- `exec --command-file`, `exec --fresh`, `exec --stream`; `doctor --fresh`.
- `fs put --checksum`, `fs get --checksum`, `fs sync --checksum`,
  `fs mirror --checksum`; `fs put --resume`, `fs get --resume`.
- `fs sync --exclude`, `fs mirror --exclude`; `fs read --lines`,
  `fs read --start`, `fs read --max-bytes`; `fs write --mode`,
  `fs write --max-bytes`; `fs patch --max-bytes`.
- `session create --shell` (bash only).

`rhost --version` is an alias of `rhost version`; both also accept `--json` and
return the same human or structured version information.

## Direct foreground execution

```bash
rhost exec <host> --command '<shell-program>'
rhost exec <host> --command-file ./inspect.sh
rhost exec <host> --cwd '<remote-directory>' --env KEY=value --command '<shell-program>'
rhost exec <host> --command '<shell-program>' --json --timeout 30s
rhost exec <host> --command-file ./inspect.sh --json --stream
rhost exec <host> --fresh --command '<shell-program>'
```

The exec form accepts exactly one non-empty shell program through `--command`
(`-c` is its short form) or `exec --command-file`. The file must be a non-empty
regular local file of at most 64 KiB and may not contain NUL; `-` is not
accepted. It is read before SSH starts and is interpreted by the same remote
login bash, without a remote temporary file or shebang dispatch. Stdin remains
available to the submitted program. Human output streams and stdin is forwarded. JSON
captures 1 MiB per stream by default; `--max-output-bytes 0` removes that
capture limit. Local flags may appear before or after the command field.

`exec --stream` requires `--json`. Remote stdout and stderr are forwarded live
to local stderr while stdout receives only the final single JSON envelope. The
envelope still captures each remote stream separately, including byte counts
and truncation flags; capture truncation does not stop the live forwarding.
Ordering between the two live streams is not guaranteed.

`exec --fresh` disables ControlMaster, ControlPersist and ControlPath for that
call. A new `bash -lc` without `--fresh` is a new execution context, not a new
SSH authentication context: sshd first starts the account's configured shell,
then rhost enters login bash. Startup files and explicit `--env` values also
shape the final environment.

Combine related read-only checks into one command when latency matters:

```bash
rhost exec <host> --command 'uname -a; free -h; df -h'
```

## JSON paths

Every JSON response has `schema_version`, `operation`, `ok`, `data` and
`error`. Use these operation paths without guessing:

| operation | useful paths |
| --- | --- |
| exec | `data.stdout`, `data.stderr`, `data.exit_code`, `data.timed_out`, `data.cancelled`, `data.cleanup_confirmed`, byte counts and truncation flags |
| connection status/reset | `data.master_status`, `data.control_path`, `data.master_pid`, `data.stopped` |
| doctor | capability fields, `data.connection`, `data.connection_reused` |
| hosts | `data.hosts[]`, `data.config_found`, `data.complete`, `data.warnings[]` |
| session create/list | `data.session_id`, `data.sessions[].session_id` |
| session exec | `data.session_id`, `data.session_ref`, `data.stdout`, `data.exit_code`, `data.session_preserved` |
| session read/recover | `data.session_id`, `data.session_ref`, content/recovery fields |
| tunnel open/list | `data.tunnel_id`, `data.status`; `data.tunnels[].tunnel_id`, `data.tunnels[].status` |
| audit | `data.entries[]` |

For exec, inspect `data.timed_out`, `data.cancelled` and
`data.cleanup_confirmed` before deciding whether a side effect is safe to retry.
When `data.cancelled` is true, `data.cancel_signal` names the local signal;
the field is absent when the command was not cancelled. An `exit_code` of `-1`
means rhost did not observe a remote command status, so classify the outcome
from `error.code` and the timeout/cancellation fields instead.
Compare `data.stdout_bytes` and `data.stderr_bytes` with the captured strings,
and check `data.stdout_truncated` and `data.stderr_truncated`. A true truncation
flag means the captured text is incomplete even though the byte count describes
the full stream. Raise the limit deliberately or redirect large remote output
to a file; do not make decisions from a truncated stream.

`session exec` uses a PTY, so `data.stdout` is the combined pane output.
`session.read` content is UTF-8.

## Hosts and diagnosis

```bash
rhost hosts --json
rhost connection status <host> --json
rhost doctor <host> --fresh --json
rhost connection reset <host> --json
```

`hosts` reads SSH client configuration without contacting a host. An empty
`hosts` array with `complete:true` means discovery succeeded but found no
concrete aliases. `complete:false` means an included config could not be read.
Any target accepted by OpenSSH may still be passed directly.

Connection status checks only rhost's ordinary shared master and reports
`alive`, `absent`, or `unknown`; it does not start the remote login shell.
Reset uses OpenSSH's stop operation, so the master stops accepting new requests
while already accepted channels continue. Dedicated tunnel masters are outside
this control. Recovery after suspected cached authentication state is: inspect
status, verify through `doctor --fresh`, reset, then independently check the
earlier operation's result. Never replay the original mutation as diagnosis.

## Persistent sessions

```bash
rhost session create <host> --json --name debug --cwd '<remote-directory>'
rhost session create <host> --shell bash --json
rhost session list <host> --json
rhost session exec <host> debug --command '<shell-program>' --json
rhost session send <host> debug --data 'python3 -i' --enter
rhost session send <host> debug --data 'next()' --enter
rhost session send <host> debug --key C-c
rhost session read <host> debug --json --since 0
rhost session recover <host> debug --json
rhost session attach <host> debug
rhost session close <host> debug
```

`--data` sends bytes verbatim: the two characters `\n` remain a backslash and
an `n`. `--enter` pastes the data and presses Enter in the same remote helper.
`SESSION_BUSY` means another program owns the pane; drive it with send/read or
recover it explicitly.
`session create --shell` currently accepts only `bash`; start another
interactive program later with send/exec. `initial_cwd` records the requested
value (including literal `~`); use `pwd` inside the session to observe its actual
working directory. Session exec/read/recover return the resolved canonical
`session_id` and preserve the caller's name or ID in `session_ref`.

## Files

```bash
rhost fs put <host> ./local '<remote-path>'
rhost fs put <host> ./local '<remote-path>' --parents
rhost fs put <host> ./local '<remote-path>' --checksum --resume
rhost fs get <host> '<remote-path>' ./local
rhost fs get <host> '<remote-path>' ./local --checksum --resume
rhost fs sync <host> ./directory '<remote-directory>' --dry-run --json
rhost fs sync <host> ./directory '<remote-directory>' --checksum --exclude '*.tmp'
rhost fs mirror <host> '<remote-directory>' ./directory --dry-run --json
rhost fs mirror <host> '<remote-directory>' ./directory --checksum --exclude '*.tmp'
rhost fs read <host> '<remote-file>' --lines 200 --start 1 --max-bytes 1048576 --json
rhost fs write <host> '<new-remote-file>' --from ./file --parents --json
rhost fs write <host> '<new-remote-file>' --from ./file --mode 0644 --max-bytes 8388608 --json
rhost fs write <host> '<existing-remote-file>' --from ./file --if-hash <sha256> --json
rhost fs patch <host> '<remote-file>' --patch ./patch.json --json
rhost fs patch <host> '<remote-file>' --patch ./patch.json --max-bytes 8388608 --json
```

Write remote paths as `'~/path'`. The outer quotes prevent local expansion;
quote characters are not part of the path. Put and write do not create missing
parents unless `--parents` is supplied.

Creating a new file needs no hash. Replacing an existing file and every patch
require the SHA-256 returned by the last `fs read`. Directory deletion requires
`--delete`; preview it with `--dry-run`.

File transfers have a default five-minute deadline. Put/get `--checksum` adds a
SHA-256 content comparison after transport; `checksum_verified:false` means
that extra comparison was not performed, not that SSH lacked integrity.
`--resume` selects resumable transfer behavior and reports whether it was
enabled. Sync/mirror checksum behavior belongs to rsync's content comparison;
none of these flags proves that a script is trustworthy or authorized.

For a local script whose stdin is not needed, shell redirection remains valid:
`rhost exec <host> --command 'bash -s' < script.sh`. Prefer `--command-file`
when the script itself is the program and stdin belongs to that program. If a
real remote script file is required, use `fs put`, execute the named path, then
remove it explicitly after checking the result.

`fs batch <host> --manifest <file>` is retained for existing callers only.
It serially runs put/get entries and reports `data.items[]`, `data.succeeded`
and `data.failed`. A completed batch has `ok:true` even with failed entries
(process status 255). It provides no rollback or isolation; prefer separate
put/get invocations for new orchestration.

## Tunnels and audit

```bash
rhost tunnel open <host> --kind local --listen localhost:8080 --destination localhost:8000 --json
rhost tunnel list --json
rhost tunnel close <id> --json
rhost audit --json
```

Tunnels bind loopback by default and use a dedicated OpenSSH master. A
`data.status` or `data.tunnels[].status` value of `"alive"` means the forward
exists; it does not probe the destination service. Audit
records bounded local operation metadata and can be disabled with
`RHOST_AUDIT=0`. Keep `data.tunnel_id` from `tunnel open`; `data.id` is an
identical compatibility alias for older callers.
