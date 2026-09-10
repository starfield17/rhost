[← Architecture map](../ARCHITECTURE.md)

# Part V — `exec`

## 10. CLI surface

Target shape:

```bash
rhost exec <host> [flags] -- <command...>
```

Examples:

```bash
rhost exec gpu -- pwd

rhost exec gpu \
  --cwd ~/work/foo \
  --timeout 30s \
  -- pytest -q

rhost exec gpu \
  --env CUDA_VISIBLE_DEVICES=0 \
  --json \
  -- python scripts/check.py
```

### Semantics

Each call gets a fresh remote execution context.

Returned model:

```json
{
  "schema_version": 1,
  "operation": "exec",
  "ok": false,
  "host": "gpu",
  "data": {
    "exit_code": 1,
    "stdout": "...",
    "stderr": "...",
    "timed_out": false,
    "duration_ms": 831
  },
  "error": null
}
```

`ok` means the adapter operation completed as designed. The remote command may still have a non-zero `exit_code`.

For shell scripting, the CLI process should normally mirror the remote command's exit status when execution reached the remote process. Infrastructure/adapter failures should use a documented separate convention.

The JSON document remains the authoritative diagnosis.

---

## 11. Foreground timeout

Foreground operations must have an upper bound.

A reasonable default may exist for humans, but the Skill should teach agents to set explicit timeouts for uncertain operations.

When a task is expected to outlive a foreground timeout, the correct action is:

```text
use `job`, not a larger and larger `exec` timeout
```

Cancellation must attempt to terminate the remote foreground process, not only kill the local `ssh` subprocess.

Design and test this explicitly; OpenSSH process termination alone is not sufficient evidence that the remote child died.

---

