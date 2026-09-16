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

### Why the edit helper is an embedded interpreter script

`fs read`/`write`/`patch` run a helper on the remote host. It is a Python program
compiled into the rhost binary and executed by the target's own `python3`; rhost
installs nothing there (AGENTS.md §5). The session helpers are generated shell
scripts instead, so this difference is deliberate:

- the remote architecture is unknown, so a Rust helper would need per-target
  cross-compilation — which the release contract forbids — and rhost never ships
  a binary to the target;
- the edit helper moves structured binary data: base64 content and hashes in one
  JSON request, one bounded JSON reply, under a `flock` and an atomic
  same-directory replace. Shell plus coreutils would need a hand-rolled quoting
  protocol to do the same;
- a target that already has `python3` supplies that JSON pump with no install; a
  target without it gets `REMOTE_DEPENDENCY_MISSING`, not a silent failure.

The mechanism is deliberately not frozen (`docs/CONTRACT.md`): the helper's
source language may change if a target without `python3` ever justifies it.

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
{"schema_version":2,"operation":"exec","ok":true,"host":"gpu","data":{"execution":{"status":"completed","exit_code":0},"cleanup":{"status":"not_attempted"},"output":{"kind":"streams","stdout":{"content":"","bytes":0,"truncated":false},"stderr":{"content":"","bytes":0,"truncated":false}},"duration_ms":1},"error":null}
```

The top-level envelope is fixed. Operation data uses these canonical paths:

| Operation | Identity | Output or collection |
| --- | --- | --- |
| exec | — | `data.execution`, `data.cleanup`, `data.output.stdout`, `data.output.stderr` |
| session create/list | `session_id` | `data.sessions[]` for list |
| session exec | `data.session_id`, `data.session_ref` | `data.execution`, `data.output.kind == "pty"`, `data.output.content` |
| session recover | `data.session_id`, `data.session_ref` | `data.session_preserved`, `data.foreground` when busy |
| tunnel open/list | `data.tunnel_id` / `data.tunnels[].tunnel_id` | `data.status` / `data.tunnels[].status` |
| session read | `data.session_id`, `data.session_ref` | `data.content`, `data.encoding == "utf-8"` |
| hosts | — | `data.hosts[]`, discovery metadata |
| audit | — | `data.entries[]` |

Session exec accepts completion only with an invocation-specific token, the
canonical ID resolved by the remote helper, and a valid exit status. Because the
command runs in a tmux PTY, `data.output.content` is merged terminal output and
cannot be split into stdout and stderr; `data.output.kind` is always `"pty"`.
Unknown execution omits an exit code rather than inventing zero; malformed helper results return
`SESSION_UNHEALTHY`. Exec refuses a foreground REPL with
`SESSION_BUSY`. Recover sends Ctrl-C under the same writer lock as exec/send and
requires a fresh shell prompt; a REPL that catches the interrupt remains running,
with `SESSION_BUSY`, `data.foreground` and `data.session_preserved:false`.
Use explicit REPL input to exit it; recover never types an exit command.
`session read` retains raw terminal content rather than removing apparent echoes.

Incremental reads also carry `from`, `next` and `more`. Agents branch on
`error.code`, never on `error.message`. The authoritative Rust machine-readable
contract is `schemas/result-v2.schema.json`. The frozen v1 schema remains wire
history only; current command availability is defined by `rhost --help`.
`session attach` is recognized but always returns a v2 `USAGE_ERROR`.
