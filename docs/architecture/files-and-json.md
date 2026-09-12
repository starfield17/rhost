# Files and JSON contracts

## Remote paths

A remote path is one argv value. Quote it for the local shell without adding
quote characters to the value:

```bash
rhost fs put gpu ./local.txt '~/work/local.txt'
rhost fs read gpu '~/.config/example'
```

The remote helper expands a leading `~`. scp and rsync paths receive equivalent
protection through their own argument construction. Paired outer single or double
quotes that arrive as part of a remote path are rejected with `CONFIG_INVALID`
before any remote operation, including `--parents`. Internal quote characters
remain literal; paths are not reinterpreted as shell syntax.

`fs put` and `fs write` do not create parent directories by default. Add
`--parents` when directory creation is intended.

## File operations

Plain `put/get` use scp. Resume and checksum modes use rsync and verify the
remote digest. Directory `sync/mirror` never deletes without `--delete`.

`fs read` returns bounded UTF-8 text and the SHA-256 of the complete file.
`fs write` may create a new file without `--if-hash`; replacing an existing
file requires the hash last returned by `fs read`. `fs patch` always requires
a matching hash. A missing required hash returns `HASH_REQUIRED`; a stale hash
returns `FILE_CONFLICT`. Replacement is locked, same-directory and atomic.

## JSON envelope

Every `--json` command emits one line:

```json
{"schema_version":1,"operation":"exec","ok":true,"host":"gpu","data":{},"error":null}
```

The top-level envelope is fixed. Operation data uses these canonical paths:

| Operation | Identity | Output or collection |
| --- | --- | --- |
| direct/compat exec | — | `data.stdout`, `data.stderr`, `data.exit_code` |
| session create/list | `session_id` | `data.sessions[]` for list |
| session exec | `data.session_id` | `data.stdout`, `data.exit_code` (integer or null) |
| session recover | `data.session_id` | `data.session_preserved`, `data.foreground` when busy |
| tunnel open/list | `data.tunnel_id` / `data.tunnels[].tunnel_id` | `id` remains an identical compatibility alias |
| session read | `data.session_id` | `data.content`, `data.encoding == "utf-8"` |
| job start/status | `data.job_id` | `data.exit_code` is integer or null |
| job list | `data.jobs[].job_id` | `data.jobs[]` |
| job logs | `data.job_id` | `data.content`, `data.encoding == "base64"` |
| hosts | — | `data.hosts[]`, discovery metadata |
| audit | — | `data.entries[]` |

Session exec accepts completion only with an invocation-specific token and a
valid exit status. Unknown exit status is `null`, never zero; malformed helper
results return `SESSION_UNHEALTHY`. Exec refuses a foreground REPL with
`SESSION_BUSY`. Recover sends Ctrl-C under the same writer lock as exec/send and
requires a fresh shell prompt; a REPL that catches the interrupt remains running,
with `SESSION_BUSY`, `data.foreground` and `data.session_preserved:false`.
Use explicit REPL input to exit it; recover never types an exit command.
`session read` retains raw terminal content rather than removing apparent echoes.

Incremental reads also carry `from`, `next` and `more`. Agents branch on
`error.code`, never on `error.message`. The authoritative machine-readable
contract is `schemas/result-v1.schema.json`.
