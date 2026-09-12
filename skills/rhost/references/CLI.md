# rhost CLI reference

## Direct foreground execution

```bash
rhost --host <host> -- '<command>'
rhost --host <host> --cwd '<remote-directory>' --env KEY=value -- '<command>'
rhost --host <host> --json --timeout 30s -- '<command>'
```

The direct form accepts exactly one shell string after `--`. Human output
streams and stdin is forwarded. JSON captures 1 MiB per stream by default;
`--max-output-bytes 0` removes that capture limit. The compatibility
`rhost exec <host> -- <command...>` form remains available with its historical
60-second deadline.

Combine related read-only checks into one command when latency matters:

```bash
rhost --host <host> -- 'uname -a; free -h; df -h'
```

## JSON paths

Every JSON response has `schema_version`, `operation`, `ok`, `data` and
`error`. Use these operation paths without guessing:

| operation | useful paths |
| --- | --- |
| exec | `data.stdout`, `data.stderr`, `data.exit_code`, `data.cleanup_confirmed` |
| hosts | `data.hosts[]`, `data.config_found`, `data.complete`, `data.warnings[]` |
| session create/list | `data.session_id`, `data.sessions[].session_id` |
| session exec | `data.stdout`, `data.exit_code`, `data.session_preserved` |
| session read | `data.content`, `data.encoding`, `data.next`, `data.more` |
| job start/status | `data.job_id`, `data.exit_code` |
| job list | `data.jobs[].job_id` |
| job logs | `data.content`, `data.encoding`, `data.next`, `data.more` |
| audit | `data.entries[]` |

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
rhost job start <host> --json --cwd '<remote-directory>' -- '<command>'
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
rhost session exec <host> debug --json -- '<command>'
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

## Tunnels and audit

```bash
rhost tunnel open <host> --kind local --listen localhost:8080 --destination localhost:8000 --json
rhost tunnel list --json
rhost tunnel close <id> --json
rhost audit --json
```

Tunnels bind loopback by default and use a dedicated OpenSSH master. `alive`
means the forward exists; it does not probe the destination service. Audit
records bounded local operation metadata and can be disabled with
`RHOST_AUDIT=0`.
