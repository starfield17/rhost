[← Architecture map](../ARCHITECTURE.md)

# Part X — structured output

## 32. JSON is a public compatibility surface

Agent-facing output is part of the product API.

Use a schema version from day one.

Common envelope:

```json
{
  "schema_version": 1,
  "operation": "status",
  "ok": true,
  "host": "gpu",
  "data": {},
  "error": null
}
```

Error example:

```json
{
  "schema_version": 1,
  "operation": "exec",
  "ok": false,
  "host": "gpu",
  "data": null,
  "error": {
    "code": "SSH_UNREACHABLE",
    "message": "host did not accept an SSH connection",
    "retryable": true
  }
}
```

Do not put ANSI color or progress spinners on stdout in JSON mode.

Human progress may go to stderr.

---

## 33. Error taxonomy

Start small and stable. The published set is `internal/errs.Codes()`, mirrored in
`schemas/result-v1.schema.json` and checked against it by `internal/output`'s tests,
so a code that exists in the enum but not in the schema (or the reverse) fails the
build. Agents branch on these values, never on the message text.

```text
USAGE_ERROR
CONFIG_INVALID
INTERNAL
HOST_UNKNOWN
SSH_UNREACHABLE
SSH_AUTH_FAILED
HOST_KEY_FAILED
REMOTE_DEPENDENCY_MISSING
REMOTE_COMMAND_TIMEOUT
SESSION_BUSY
SESSION_NOT_FOUND
SESSION_UNHEALTHY
JOB_NOT_FOUND
JOB_STATE_UNKNOWN
TRANSFER_FAILED
FILE_TOO_LARGE
SYNC_REJECTED
FILE_NOT_FOUND
FILE_CONFLICT
INVALID_PATCH
INVALID_TARGET
INVALID_TEXT
SEARCH_FAILED
TUNNEL_FAILED
TUNNEL_NOT_FOUND
UNSUPPORTED_REMOTE_OS
```

`SESSION_BUSY` is the answer to `session exec` on a pane whose foreground is not
the managed shell: a program owns the terminal, so the command was not run at
all. It is deliberately not `SESSION_UNHEALTHY` — there is nothing wrong with the
session — and the caller's next move is `session send`/`session read`, or
`session recover`, not a retry of the same paste.

The `FILE_*`, `INVALID_*` and `SEARCH_FAILED` codes come from the remote helper
(`internal/fileops/remote.py`) and travel back inside the envelope as JSON rather
than as text the caller has to scrape. They are *failures of the operation on the
remote*, not of SSH, which is why they are separate from `TRANSFER_FAILED`: an agent
retrying `SSH_UNREACHABLE` is reasonable, retrying `FILE_CONFLICT` is not. Because
that vocabulary is a second contract, it is whitelisted on the way in
(`errs.KnownCode`) — a remote that returns a made-up code cannot put a code into
`error.code` that the taxonomy has never heard of; such an answer is reported as
`INTERNAL` with the raw string kept in the message.

Keep low-level OpenSSH stderr available for diagnosis but do not force the agent to classify behavior by matching English error strings.

`USAGE_ERROR` is reserved for argument/flag parsing failures, which happen before any command runs. The usage path still emits a normal envelope when `--json` is requested, because every agent-visible outcome needs a machine-readable form.

OpenSSH runs at `LogLevel=ERROR` by default so that successful commands keep stderr clean. That level also hides ssh's own reason for some connection failures; `RHOST_SSH_LOG_LEVEL` raises it (for example `VERBOSE`) without rebuilding the binary, and the fallback message points there.

---
