# Recovering from a failed rhost call

Read `ok`, `error.code`, `error.retryable`, and the operation's `data`. English
messages are diagnostics, not a branching interface. Missing evidence remains
unknown rather than being converted into success or safe retry permission.

The sections below are the RECOVERY table: every `error.code` the binary can
return, grouped by cause, with what it does and does not prove. Branch on the
code, then confirm state with the operation named here. `--help` and
`rhost version --json` describe the installed binary this table belongs to.
`make check` fails if this table and the active schema's `error.code` enum
disagree, so a new code cannot ship undocumented.

## Input and internal

- `USAGE_ERROR` means the request never left the local process: a missing,
  unknown or contradictory argument, an unusable `--command`, or an operation
  the schema refuses (`session attach` needs a terminal). Correct the invocation;
  `error.operation` names the operation that was judged. Nothing remote ran, so
  nothing needs inspecting — but do not retry the same argv unchanged.
- `CONFIG_INVALID` means local input was well-formed as argv but not as a value:
  an empty or invalid `--command`, a path or `--cwd` that cannot be used, a
  malformed `--manifest`, or a transfer operand that names the wrong side. It is
  not retryable. Fix the value and resubmit; no remote state changed.
- `INTERNAL` means a local invariant failed — a nonce that could not be drawn, a
  capture or rendering step that should not fail, or a use-case result that
  reached the renderer in an impossible shape. It is not retryable and proves
  nothing about the remote side. Keep the `error.message`, check the remote state
  for the operation before resubmitting, and report it rather than editing the
  output by hand.

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
  verified transfers need rsync; remote filesystem helpers (text editing and
  destructive-sync target resolution) need Python 3.5 or newer with the
  required filesystem features. `doctor` checks command presence only; the
  helper checks readiness on the target before running. rhost never installs
  these dependencies.
- `SESSION_BUSY` means the requested exec was not submitted because another
  program owns the pane. Drive it with send/read or recover it explicitly.
- `SESSION_UNHEALTHY` means the helper, lock, pane, or completion evidence was not
  reliable. After `session send`, read the pane before repeating input. After
  `session close`, inspect `session list` before another close. Recover only
  when interruption of a still-running pane is intended.
- `SESSION_NOT_FOUND` never silently recreates state. Create a new session only
  when losing the previous state is acceptable.
- A `session create` that reached the host but did not return its full record
  reports `data.creation_status`: `not_created` (a definite refusal), `unknown`
  (a timeout, cancellation, disconnect or missing evidence), or `created`
  (creation was observed). Read `data.session_id` and check `session list`
  before retrying; a local argument mistake leaves `data` null.

For session results, use `data.execution.status`, `data.session_preserved`, and
PTY content under `data.output`. A timeout with `session_preserved:false` means
the managed shell is not proven usable.

When a dependency is missing, keep using rhost operations whose dependencies
are present. If python3 is missing or unusable, do not repeat a failed `fs read`, `fs write`,
or `fs patch` through the same helper. If bash is missing, `rhost exec` and
`rhost session` are not a shell fallback. The agent can instead use its local
command tool to run OpenSSH: prefer a one-shot `ssh` command for work that can
finish in one invocation, and use a direct SSH login in a local PTY when commands
need interactive input or stepwise inspection. Use local `scp` for a file copy
only if the host supports the required transfer service. These direct tools do
not produce rhost's JSON evidence or file-editing guarantees; see
[CLI.md](CLI.md) and [SAFETY.md](SAFETY.md).

## Files and tunnels

- `HASH_REQUIRED` asks for the SHA-256 from a fresh `fs read` before replacement.
- `FILE_CONFLICT` means the file changed; read and merge again.
- `FILE_NOT_FOUND`, `INVALID_TARGET`, `INVALID_TEXT`, `FILE_TOO_LARGE`, and
  `INVALID_PATCH` require inspecting or correcting the named input.
- An editing `INVALID_TARGET` can also mean a symlink in any path component.
  Supply the real file's path; rhost deliberately does not edit through aliases.
- `SYNC_REJECTED` is a safety refusal before transfer. On the remote side it is
  evaluated against the destination after symlink resolution. Correct the target;
  do not work around it with shell expansion.
- `TRANSFER_FAILED` is a local scp/rsync failure after an attempted transfer.
  Inspect the tool diagnostic and actual destination before retrying.
- `TUNNEL_FAILED` leaves forward state uncertain. A failed `tunnel open` may
  retain a record when its master could still be running; inspect `tunnel list`
  before opening another forward. Preserve the record until the named master
  can be checked or closed. `TUNNEL_NOT_FOUND` means no record exists for that
  canonical ID.

## Safe recovery sequence

When a shared connection looks stale, inspect `connection status`, probe with
`doctor --fresh`, then use `connection reset` if appropriate. Independently
inspect the earlier operation's result afterward. Never repeat the original
mutation merely to test connectivity.

For long work that must survive the agent runtime, submit it to a scheduler
already installed on the remote host and use that scheduler's own status and
logs. A tmux session is interactive state, not a generic scheduler.

## Reserved codes

- `UNSUPPORTED_REMOTE_OS` is in the closed `error.code` enum but the current
  binary never emits it; a host whose OS rhost cannot drive fails through the
  dependency or command path instead. Treat it as a compatibility value: if it
  ever appears, stop and report the host rather than working around it.

## Privilege, packages and login shells

rhost does not infer which package supplies a tool, and it does not manage
privilege for you. After changing packages or the login shell, re-verify command
resolution in the context that will actually run: as the ordinary user, and again
under the real `sudo`/service context that a long-running step uses, since they
can see different `PATH`s.

Do not rely on a `sudo` timestamp surviving between rhost calls; each call is a
new submission and the credential cache may be gone. Interactive `sudo` belongs
in OpenSSH's own PTY (`ssh -t <host> 'sudo ...'`), never inside a session `send`,
an argument, an environment variable or a temporary plaintext file.

After an account's state changes, run `doctor --fresh` to verify what the new
login resolves to, then `connection reset` to retire the old authentication
snapshot so the next call re-authenticates cleanly.
