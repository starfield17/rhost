[← Architecture map](../ARCHITECTURE.md)

# Part I — System shape

## 1. Top-level architecture

```text
                         Coding Agent
                              │
                         reads SKILL.md
                              │
                    shell command invocation
                              ▼
                     ┌─────────────────┐
                     │   rhost CLI     │
                     │  disposable     │
                     └────────┬────────┘
                              │
                 ┌────────────┼─────────────┐
                 │            │             │
                 ▼            ▼             ▼
          human formatting   JSON       watch/TUI
                 │            │             │
                 └────────────┼─────────────┘
                              ▼
                     ┌─────────────────┐
                     │ application     │
                     │ use-cases       │
                     └────────┬────────┘
                              │
          ┌───────────────────┼────────────────────┐
          ▼                   ▼                    ▼
     transport            persistence          telemetry
     OpenSSH              adapters             probes
          │                   │                    │
          ├──────────────┬────┴─────────┬──────────┤
          ▼              ▼              ▼          ▼
       SSH exec       remote tmux   remote jobs   remote FS
          │              │              │          │
          └──────────────┴───────┬──────┴──────────┘
                                 ▼
                          Remote Linux host
```

No project-specific logic belongs below the CLI.

---

## 2. Persistence ownership

This table is an architectural contract.

| Concern | v0.1 owner | Why |
|---|---|---|
| SSH connection reuse | local OpenSSH ControlMaster | survives `rhost` process exit |
| stateless exec | fresh SSH channel/process | predictable semantics |
| persistent shell | remote tmux | survives local process/network disconnect |
| session terminal log | remote state file | allows incremental reads after reconnect |
| background job | remote process/process group | survives SSH disconnect |
| job metadata/logs | remote state directory | rediscoverable |
| host aliases/auth | `~/.ssh/config` | do not duplicate SSH |
| adapter config | local `rhost` config | project-specific defaults do not belong here |
| UI/watch state | reconstructed | monitor must not own runtime state |

Do not add in-memory ownership where a process restart would violate this table.

---

