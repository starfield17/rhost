[← Architecture map](../ARCHITECTURE.md)

# Part XIII — Portal source as reference

## 37. What Codex should inspect in local `portal-mcp-server`

The user will place the Portal source locally.

Use it as a source of implementation ideas.

In current Portal versions, useful areas are conceptually equivalent to:

```text
connection manager
session manager
remote shell engine
job manager
file operations / SFTP
security / audit
```

If the local checkout contains files such as:

```text
connection_manager.py
session_manager.py
remote_bash.py
job_manager.py
file_ops.py
security.py
audit.py
```

inspect them.

### Borrow the ideas

Especially study:

1. SSH transport reuse and channel reuse.
2. The semantic split between stateless exec and persistent shell.
3. PTY command-completion markers and exit-code recovery.
4. Shell locking / avoiding concurrent readers on one PTY.
5. Timeout, Ctrl-C soft cancellation, and resynchronization.
6. Detached background job launch.
7. Incremental job-log reads by offset.
8. Structured result/error models.
9. Transfer behavior and bounded outputs.
10. Separation of execution logic from security/audit logic.

### Do not copy the product shape

Do not port:

- FastMCP tool registration as the primary frontend;
- mandatory MCP transport;
- a long-running Python server as the owner of persistence;
- process-memory-only session registries;
- Python/asyncio-specific lifecycle assumptions.

The important difference is:

```text
Portal-style server:
server process owns SSH objects / persistent PTY

rhost v0.1:
CLI owns nothing durable
ControlMaster owns connection reuse
remote tmux owns session
remote process/state dir owns job
```

If Portal has a better implementation detail that fits this ownership model, use it.

---

