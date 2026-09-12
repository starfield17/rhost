# rhost CLI reference

## Direct foreground execution

```bash
rhost --host <host> -- '<command>'
rhost --host <host> --cwd '<remote-directory>' --env KEY=value -- '<command>'
rhost --host <host> --json --timeout 30s -- '<command>'
```

The direct form accepts exactly one shell string after `--`. Human output is
streamed and stdin is forwarded. JSON is one envelope after completion, with a
1 MiB capture limit per stream unless `--max-output-bytes` is set. A value of `0`
removes the JSON capture limit.

The compatibility form remains available:

```bash
rhost exec <host> --cwd '<remote-directory>' --timeout 60s -- <command...>
```

It joins command words with spaces and keeps the historical 60-second and 1 MiB
defaults. Prefer the direct form for new agent workflows.

## Hosts and diagnosis

```bash
rhost hosts --json
rhost doctor <host> --json
```

Any name accepted by OpenSSH can be passed directly. `hosts` is best-effort alias
discovery. `doctor` is for diagnosis; ordinary execution does not require a
preflight probe.

## Durable jobs

```bash
rhost job start <host> --json --cwd '<remote-directory>' -- '<command>'
rhost job list <host> --json
rhost job status <host> <job-id> --json
rhost job logs <host> <job-id> --json --since 0
rhost job stop <host> <job-id> --json
rhost job kill <host> <job-id> --json
```

Jobs are detached remote process groups with remote metadata and logs. They
survive the CLI and SSH connection. Log reads use byte cursors; pass `data.next`
back through `--since`. Keep the generated id and inspect state before retrying a
launch whose result was uncertain.

## Persistent sessions

```bash
rhost session create <host> --json --name debug --cwd '<remote-directory>'
rhost session list <host> --json
rhost session exec <host> debug --json -- '<command>'
rhost session send <host> debug --data 'next()\n'
rhost session send <host> debug --key C-c
rhost session read <host> debug --json --since 0
rhost session recover <host> debug --json
rhost session attach <host> debug
rhost session close <host> debug
```

Sessions live in remote tmux. Use `session exec` only when the managed shell owns
the pane. `SESSION_BUSY` means a REPL, debugger, or other foreground program owns
it; use `send` and `read`, or explicitly recover it.

## Files

```bash
rhost fs put <host> ./local '<remote-path>'
rhost fs get <host> '<remote-path>' ./local
rhost fs sync <host> ./directory '<remote-directory>' --dry-run --json
rhost fs mirror <host> '<remote-directory>' ./directory --dry-run --json
rhost fs read <host> '<remote-file>' --json
rhost fs write <host> '<remote-file>' --from ./file --if-hash <sha256> --json
rhost fs patch <host> '<remote-file>' --patch ./patch.json --json
rhost fs batch <host> --manifest ./transfers.json --json
```

`read` returns the whole-file SHA-256 alongside bounded text. Replacement and
patching require that hash, reject symlink targets, and replace atomically in the
same directory. Transfers use scp or rsync. Directory deletion requires explicit
`--delete`; preview it with `--dry-run`.

Use direct execution for remote search:

```bash
rhost --host <host> --cwd '<remote-directory>' -- 'rg TODO src'
```

## Tunnels and audit

```bash
rhost tunnel open <host> --kind local --listen localhost:8080 --destination localhost:8000 --json
rhost tunnel list --json
rhost tunnel close <id> --json
rhost audit --json
```

Tunnels bind loopback by default and use a dedicated OpenSSH master that survives
the opening command. Audit records bounded operation metadata locally and can be
disabled with `RHOST_AUDIT=0`.
