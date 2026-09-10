[← Architecture map](../ARCHITECTURE.md)

# Part XI — SKILL.md

## 34. Required Skill behavior

The repository should ship a concise `SKILL.md`.

Its main rules should be operational facts, not general software-engineering advice.

Suggested behavior:

```text
Use rhost when work must execute on an SSH-configured remote host.

Before first use on a host:
  rhost doctor <host> --json

Default:
  rhost exec ... --json

Use session only when:
  cwd/env/interactive terminal state must persist.

Use job when:
  the task must survive disconnect,
  or the expected runtime is longer than a foreground operation.

For files:
  use rhost fs ...
  use dry-run before sync deletion.

For machine state:
  use rhost status --json.
  `rhost watch` is for humans, not for agent parsing.

After a disconnect:
  rediscover session/job state; do not assume it vanished.

Never parse the human TUI when a JSON form exists.
```

The Skill should name actual CLI commands and behaviors.

It should not contain a tutorial on SSH, tmux, Go, or system design.

---

