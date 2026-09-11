[← Architecture map](../ARCHITECTURE.md)

# Part XI — the skill

## 34. Required Skill behavior

The skill ships as an installable directory, `skills/rhost/`:

```text
skills/rhost/
├── SKILL.md            frontmatter, hard rules, routing
└── references/
    ├── CLI.md          the command surface
    ├── RECOVERY.md     error codes and what each asks for
    └── SAFETY.md       authority, secrets, destructive operations
```

`SKILL.md` carries YAML frontmatter (`name`, `description`) so an agent can list
it before loading it, and stays short: the rules nobody should have to
rediscover, plus a table pointing at the reference the current task needs.
Detail that only some tasks need lives in `references/` — progressive
disclosure, not a smaller manual. Nothing binary is ever stored in the skill
directory: the executable comes from a GitHub Release.

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
