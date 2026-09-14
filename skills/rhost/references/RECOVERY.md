# Recovering from a failed rhost call

Read `ok`, `error.code`, `error.retryable`, and the operation's `data`. English
messages are diagnostics, not a branching interface. Missing evidence remains
unknown rather than being converted into success or safe retry permission.

## Execution and transport

- `SSH_UNREACHABLE` proves a connection-stage failure and may be retryable, but
  never proves an earlier side effect is safe to replay.
- `SSH_AUTH_FAILED`, `HOST_KEY_FAILED`, and `HOST_UNKNOWN` require correcting
  local OpenSSH identity, trust, or target configuration. Never disable host-key
  checking.
- `SSH_CONTROL_FAILED` leaves shared-master state unknown. Inspect it again; do
  not delete sockets manually.
- `REMOTE_EXECUTION_UNKNOWN` means no invocation-bound completion was observed.
  It is not known success and is not retryable.
- `REMOTE_COMMAND_TIMEOUT` and `REMOTE_COMMAND_CANCELLED` may have remote effects.
  `data.cleanup.status == "confirmed_stopped"` proves only that the matched
  managed process group stopped; it does not undo effects or cover detached work.
- `OUTPUT_WRITE_FAILED` means local delivery failed. Preserve any available
  execution evidence and inspect remote state before another mutation.

## Dependencies and sessions

- `REMOTE_DEPENDENCY_MISSING` names a feature dependency. Exec needs remote bash,
  setsid, ps, and a base64 decoder; sessions need tmux and flock; sync/mirror and
  verified transfers need rsync; text editing needs python3. rhost never installs
  them.
- `SESSION_BUSY` means the requested exec was not submitted because another
  program owns the pane. Drive it with send/read or recover it explicitly.
- `SESSION_UNHEALTHY` means the helper, lock, pane, or completion evidence was not
  reliable. Read the pane first, then recover if interruption is intended.
- `SESSION_NOT_FOUND` never silently recreates state. Create a new session only
  when losing the previous state is acceptable.

For session results, use `data.execution.status`, `data.session_preserved`, and
PTY content under `data.output`. A timeout with `session_preserved:false` means
the managed shell is not proven usable.

## Files and tunnels

- `HASH_REQUIRED` asks for the SHA-256 from a fresh `fs read` before replacement.
- `FILE_CONFLICT` means the file changed; read and merge again.
- `FILE_NOT_FOUND`, `INVALID_TARGET`, `INVALID_TEXT`, `FILE_TOO_LARGE`, and
  `INVALID_PATCH` require inspecting or correcting the named input.
- `SYNC_REJECTED` is a local safety refusal before transfer. Correct the target;
  do not work around it with shell expansion.
- `TRANSFER_FAILED` is a local scp/rsync failure after an attempted transfer.
  Inspect the tool diagnostic and actual destination before retrying.
- `TUNNEL_FAILED` leaves forward state uncertain. Preserve the record and inspect
  again. `TUNNEL_NOT_FOUND` means no record exists for that canonical ID.

## Safe recovery sequence

When a shared connection looks stale, inspect `connection status`, probe with
`doctor --fresh`, then use `connection reset` if appropriate. Independently
inspect the earlier operation's result afterward. Never repeat the original
mutation merely to test connectivity.

For long work that must survive the agent runtime, submit it to a scheduler
already installed on the remote host and use that scheduler's own status and
logs. A tmux session is interactive state, not a generic scheduler.
