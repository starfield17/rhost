[← Architecture map](../ARCHITECTURE.md)

# Part XII — security and audit

## 35. Security stance

`rhost` is a remote execution adapter, not a sandbox.

Its authority equals the configured SSH identity.

Hard requirements:

- never disable host-key checking globally;
- never persist private keys;
- never persist passwords;
- default agent use to non-interactive authentication;
- do not silently elevate privileges;
- do not auto-install remote dependencies;
- keep managed remote state user-private;
- redact known secret-bearing environment values from logs/metadata when such a mechanism is added.

Do not claim that a command is safe merely because it ran through `rhost`.

---

## 36. Audit trail

A lightweight local audit log is useful:

```text
~/.local/state/rhost/audit.jsonl
```

Record operations, not secrets:

```json
{
  "time": "...",
  "host": "gpu",
  "operation": "exec",
  "cwd": "/home/dev/work/foo",
  "command_summary": "pytest -q",
  "exit_code": 1,
  "duration_ms": 831
}
```

Do not record full environment maps by default.

Do not make audit logging a prerequisite for v0.1 execution if the design would make a full disk brick all remote work; choose and document fail-open/fail-closed behavior consciously.

---

