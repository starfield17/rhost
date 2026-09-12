[← Architecture map](../ARCHITECTURE.md)

## 0. Implementation directive

Extend an agent's existing command-line behavior to an SSH-reachable host.

The core flow is:

```text
command + remote context → OpenSSH → stdin/stdout/stderr/exit status
```

The most important invariant is:

> **Anything promised to survive a CLI invocation must be owned outside the CLI process.**

The CLI may exit after every command.

Do not add a dedicated operation for behavior an ordinary remote command can
compose. Do not solve persistence by keeping a hidden `rhost` server alive.

---
