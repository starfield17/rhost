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

Start small and stable.

Candidate codes:

```text
USAGE_ERROR
CONFIG_INVALID
HOST_UNKNOWN
SSH_UNREACHABLE
SSH_AUTH_FAILED
HOST_KEY_FAILED
REMOTE_DEPENDENCY_MISSING
REMOTE_COMMAND_TIMEOUT
SESSION_NOT_FOUND
SESSION_UNHEALTHY
JOB_NOT_FOUND
JOB_STATE_UNKNOWN
TRANSFER_FAILED
SYNC_REJECTED
UNSUPPORTED_REMOTE_OS
```

Keep low-level OpenSSH stderr available for diagnosis but do not force the agent to classify behavior by matching English error strings.

`USAGE_ERROR` is reserved for argument/flag parsing failures, which happen before any command runs. The usage path still emits a normal envelope when `--json` is requested, because every agent-visible outcome needs a machine-readable form.

OpenSSH runs at `LogLevel=ERROR` by default so that successful commands keep stderr clean. That level also hides ssh's own reason for some connection failures; `RHOST_SSH_LOG_LEVEL` raises it (for example `VERBOSE`) without rebuilding the binary, and the fallback message points there.

---

