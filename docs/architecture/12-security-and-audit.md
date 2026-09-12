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

Status: implemented. `rhost` appends one Entry per audited remote operation to
`$XDG_STATE_HOME/rhost/audit.jsonl` (default `~/.local/state/rhost/audit.jsonl`,
overridable with `RHOST_STATE_DIR`), and `rhost audit [--json] [--limit N]
[--host H]` reads it back.

- **Fail-open**, chosen consciously: a write error is reported on stderr and the
  operation proceeds, so a full or read-only disk cannot brick remote work.
- Records bounded operation metadata: no environment maps and no file contents, and
  `session send` records the key but never the injected data.
- Audited operations are direct and compatibility execution, `doctor`, session
  and job mutations, file writes/transfers, and tunnel mutations. List, read,
  and log-poll commands are not audited, so the trail records actions rather
  than polling noise.
- Audit state is local-only. Do not point `RHOST_STATE_DIR` at a repository or
  upload `audit.jsonl`; the repository ignore rules provide a final backstop.
- Turned off with `RHOST_AUDIT=0`.

---
