# rhost CLI reference

## Direct foreground execution

```bash
rhost exec <host> --command '<shell-program>'
rhost exec <host> --cwd '<remote-directory>' --env KEY=value --command '<shell-program>'
rhost exec <host> --command '<shell-program>' --json --timeout 30s
```

The exec form accepts exactly one non-empty shell program through `--command`
(`-c` is its short form). Human output streams and stdin is forwarded. JSON
captures 1 MiB per stream by default; `--max-output-bytes 0` removes that
capture limit. Local flags may appear before or after the command field.

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
| hosts | `data.hosts[]`, `data.config_found`, `data.complete`, `data.warnings[]` |
| session create/list | `data.session_id`, `data.sessions[].session_id` |
| session exec | `data.stdout`, `data.exit_code`, `data.session_preserved` |
| session read | `data.content`, `data.encoding`, `data.next`, `data.more` |
| job start/status | `data.job_id`, `data.exit_code` |
| job list | `data.jobs[].job_id` |
| job logs | `data.content`, `data.encoding`, `data.next`, `data.more` |
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
`session.read` content is UTF-8. `job.logs` content is base64 because job
output may contain arbitrary bytes. A job without a recorded final status has
`exit_code:null`.

## Hosts and diagnosis

```bash
rhost hosts --json
rhost doctor <host> --json
```

`hosts` reads SSH client configuration without contacting a host. An empty
`hosts` array with `complete:true` means discovery succeeded but found no
concrete aliases. `complete:false` means an included config could not be read.
Any target accepted by OpenSSH may still be passed directly.

## Durable jobs

```bash
rhost job start <host> --command '<shell-program>' --json --cwd '<remote-directory>'
rhost job list <host> --json
rhost job status <host> <job-id> --json
rhost job logs <host> <job-id> --json --since 0
rhost job stop <host> <job-id> --json
rhost job kill <host> <job-id> --json
```

Jobs are remote process groups with remote metadata and logs. Pass
`data.next` from a log response back through `--since`.

## Persistent sessions

```bash
rhost session create <host> --json --name debug --cwd '<remote-directory>'
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

## Files

```bash
rhost fs put <host> ./local '<remote-path>'
rhost fs put <host> ./local '<remote-path>' --parents
rhost fs get <host> '<remote-path>' ./local
rhost fs sync <host> ./directory '<remote-directory>' --dry-run --json
rhost fs mirror <host> '<remote-directory>' ./directory --dry-run --json
rhost fs read <host> '<remote-file>' --json
rhost fs write <host> '<new-remote-file>' --from ./file --parents --json
rhost fs write <host> '<existing-remote-file>' --from ./file --if-hash <sha256> --json
rhost fs patch <host> '<remote-file>' --patch ./patch.json --json
```

Write remote paths as `'~/path'`. The outer quotes prevent local expansion;
quote characters are not part of the path. Put and write do not create missing
parents unless `--parents` is supplied.

Creating a new file needs no hash. Replacing an existing file and every patch
require the SHA-256 returned by the last `fs read`. Directory deletion requires
`--delete`; preview it with `--dry-run`.

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
