---
name: rhost
description: "Operate SSH-reachable hosts with rhost: execute recoverable work, inspect or edit files, and drive persistent terminals through available MCP tools or the installed CLI. Use when work belongs remotely and OpenSSH should own authentication and host keys."
---

# rhost

Use the user's SSH target: alias, `user@example-host`, or hostname. Discovery
lists local OpenSSH aliases; it is best-effort, not a host inventory.

Choose the operation by its guarantees, then use its available interface.
MCP tools take structured arguments; CLI provides streaming foreground exec,
transfers and tunnels. Use the names/schemas advertised by the connected server.
Missing tools can mean an older binary or a configured tool group; do not repeat an uncertain mutation
through another interface to test whether it worked.

## Read only the guide needed now

| Task | Operation and reference |
| --- | --- |
| Ordinary command, script, stdin or streaming output | `rhost exec`; [CLI.md](references/CLI.md#foreground-execution) |
| Build/test that must survive CLI/MCP exit | Run start/poll/signal; [RUNS.md](references/RUNS.md) |
| Read/search files, CAS edit, named changeset recovery | [FILES.md](references/FILES.md) |
| Stateful shell, REPL or debugger | Session create/send/read; [SESSIONS.md](references/SESSIONS.md) |
| Connect MCP or send one JSON request | [MCP.md](references/MCP.md) |
| Diagnose SSH/dependencies, copy/sync or forward ports | Relevant section of [CLI.md](references/CLI.md) |
| Helper cache/build policy or preparation failure | [NATIVE.md](references/NATIVE.md#preparation-policies) |

Native file operations prepare their helper automatically. Runs need helper
preparation's generation/time and a caller-retained key before submission.
Doctor diagnoses shell tools; it is not a prerequisite or readiness check for
native operations.

## Keep the evidence

- Use CLI `--json` for hashes and programmatic decisions. MCP returns the same
  schema-v4 envelope in `structuredContent`. Branch on `error.code` and the
  operation's completion/mutation evidence plus `error.recovery`, never English
  messages alone.
- Replacing or patching a file requires an observed hash. Re-read and merge on
  conflict; a changeset has no global atomic visibility or automatic rollback.
- Run wait/cancellation ends observation. Session exec timeout may interrupt its
  pane. Before retrying an uncertain mutation or input, read the relevant section
  of [RECOVERY.md](references/RECOVERY.md); changing interfaces adds no retry safety.
- Output cursors count source bytes. Advance with the returned cursor and inspect
  truncation/gaps; rendered text length is not an offset.
- OpenSSH owns credentials/trust. For destructive sync, exposure, sudo or account
  changes, read [SAFETY.md](references/SAFETY.md). Manage project/Git state yourself.

If behavior differs from these guides, check `rhost version --json` and
`rhost <command> --help`, or the connected MCP tool list. A symlinked skill can
drift from the installed binary. Do not preload every reference.
