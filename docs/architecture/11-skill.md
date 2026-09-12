[← Architecture map](../ARCHITECTURE.md)

# Part XI — the skill

## 34. Required Skill behavior

The installable `skills/rhost/` directory contains a short routing document and
separate CLI, recovery, and safety references. It contains no binary.

The skill teaches one default:

```bash
rhost --host <host> --cwd '<remote-directory>' -- '<ordinary shell command>'
```

It must not make an agent choose among specialized commands before ordinary
execution. `doctor` is diagnostic rather than mandatory preflight. `job`,
`session`, `fs`, and `tunnel` are introduced only by the guarantee a task needs.

The skill names exact implemented commands, prefers JSON when structured state is
needed, and tells agents to branch on error codes. It does not duplicate SSH,
tmux, Go, or implementation documentation.
