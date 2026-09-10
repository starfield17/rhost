[← Architecture map](../ARCHITECTURE.md)

## 0. Implementation directive

Build a general remote-host adapter, not a YOLO tool and not an SSH wrapper collection.

The core model is:

```text
RemoteHost
├── Exec       stateless foreground command
├── Session    persistent interactive terminal
├── Job        durable background process
├── FS         file transfer / synchronization
└── Status     generic host telemetry and managed-state discovery
```

The most important invariant is:

> **Anything promised to survive a CLI invocation must be owned outside the CLI process.**

The CLI may exit after every command.

Do not solve persistence by keeping a hidden `rhost` server alive in v0.1.

---
