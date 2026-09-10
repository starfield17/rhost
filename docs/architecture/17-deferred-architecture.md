[← Architecture map](../ARCHITECTURE.md)

# Part XVII — deferred architecture

## 50. When a local daemon becomes justified

Do not add `rhostd` merely to make the architecture look service-oriented.

A daemon becomes justified if real usage needs one or more of:

- very high-frequency telemetry;
- continuous bidirectional PTY streaming;
- multiple local clients coordinating writes;
- event subscriptions rather than polling;
- richer human takeover semantics;
- persistent native SSH connections that outperform ControlMaster materially;
- local API consumers beyond the CLI.

Then the shape may become:

```text
Agent / Human
     │
   rhost CLI
     │ Unix socket
     ▼
   rhostd
     │
     ▼
Remote hosts
```

Even then, sessions/jobs should not become dependent on daemon survival unless intentionally redesigned.

---

## 51. When a remote runtime becomes justified

A remote daemon is a later step, not v0.1.

It becomes justified if tmux/nohup start limiting:

- precise PTY streaming;
- reliable process trees;
- resource accounting;
- event delivery;
- job isolation;
- Windows-native remote support;
- many concurrent clients.

A future `rhost-agentd` could expose a private protocol through an SSH tunnel.

SSH should remain the authentication/network boundary unless there is a compelling reason to replace it.

---

