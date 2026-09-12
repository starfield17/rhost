[← Architecture map](../ARCHITECTURE.md)

# Part XX — Codex self-check

Before declaring an implementation task complete, ask:

- Did this change preserve the three distinct execution semantics?
- Could this behavior be an ordinary remote command instead of a new rhost operation?
- Does any promised persistent state die with the CLI process?
- Is OpenSSH still the source of truth for auth and host verification?
- Did I add a project-specific concept to a generic adapter?
- Does the agent have a JSON path for information I rendered for humans?
- Can reconnect rediscover the truth instead of relying on local memory?
- Can I deliberately break the feature and see the relevant test fail?
- Did I borrow an idea from Portal without accidentally borrowing its MCP/server lifecycle?
- Did I add abstraction before a second implementation required it?
- Did I test on the real client → remote Linux/WSL2 path for behavior that mocks cannot prove?

If a persistence feature has not survived a real process exit and reconnect, it is not implemented yet.
