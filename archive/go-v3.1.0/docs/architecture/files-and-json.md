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

## Batch maintenance boundary

`fs batch` is serial CLI orchestration over existing put/get operations, not a
new transfer guarantee. Its retained value is an ordered per-entry report in
one invocation. OpenSSH multiplexing already serves independent invocations;
batch does not add isolation, atomicity, rollback, or durable execution.

Repository evidence consists of manifest validation tests, live transfer tests,
and recovery guidance, not evidence of external usage. Retain the published
manifest and aggregate semantics for compatibility; prefer put/get for new
callers. Do not add concurrency, dependency graphs, rollback or a versioned
workflow language without demonstrated requirements that existing operations
cannot meet. Removal would require a major-version compatibility review.

A completed run emits `ok:true`; inspect `data.failed` and each `data.items[]`
entry. Any entry failure sets process status 255; subsequent entries still run.
Invalid manifests fail before transfers begin. This differs deliberately from
single-transfer success and must not be silently normalized.

## JSON envelope

Every `--json` command emits one line:

```json
{"schema_version":1,"operation":"exec","ok":true,"host":"gpu","data":{},"error":null}
```

The top-level envelope is fixed. Operation data uses these canonical paths:

| Operation | Identity | Output or collection |
| --- | --- | --- |
| exec | — | `data.stdout`, `data.stderr`, `data.exit_code` |
| session create/list | `session_id` | `data.sessions[]` for list |
| session exec | `data.session_id`, `data.session_ref` | `data.stdout` (PTY output), `data.output_kind == "pty"`, `data.exit_code` (integer or null) |
| session recover | `data.session_id`, `data.session_ref` | `data.session_preserved`, `data.foreground` when busy |
| tunnel open/list | `data.tunnel_id` / `data.tunnels[].tunnel_id` | `id` remains an identical compatibility alias |
| session read | `data.session_id`, `data.session_ref` | `data.content`, `data.encoding == "utf-8"` |
| hosts | — | `data.hosts[]`, discovery metadata |
| audit | — | `data.entries[]` |

Session exec accepts completion only with an invocation-specific token, the
canonical ID resolved by the remote helper, and a valid exit status. Its
`data.stdout` name is a compatibility field: because the
command runs in a tmux PTY, it contains merged terminal output and cannot be
split into stdout and stderr; `data.output_kind` is always `"pty"`. Unknown exit
status is `null`, never zero; malformed helper results return
`SESSION_UNHEALTHY`. Exec refuses a foreground REPL with
`SESSION_BUSY`. Recover sends Ctrl-C under the same writer lock as exec/send and
requires a fresh shell prompt; a REPL that catches the interrupt remains running,
with `SESSION_BUSY`, `data.foreground` and `data.session_preserved:false`.
Use explicit REPL input to exit it; recover never types an exit command.
`session read` retains raw terminal content rather than removing apparent echoes.

Incremental reads also carry `from`, `next` and `more`. Agents branch on
`error.code`, never on `error.message`. The authoritative machine-readable
contract is `schemas/result-v1.schema.json`. That schema is retained as a
wire-history contract and includes operation variants emitted by earlier CLI
majors; current command availability is defined by `rhost --help`.
