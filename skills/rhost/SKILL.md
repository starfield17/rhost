---
name: rhost
description: Run commands and perform explicit file, session, tunnel, or diagnostic operations on SSH-reachable hosts through an installed rhost CLI. Use when work belongs on a remote host and OpenSSH should retain authentication and host-key ownership.
---

# rhost

Use the same shell program you would run locally and add the remote context:

```bash
rhost exec <host> --cwd '<remote-directory>' --command '<shell-program>'
```

The `--command` value, or the contents of `--command-file`, is one program for a
remote login bash. Shell syntax inside that value is remote; a pipeline outside
the quoted argument is local. Prefer `--json` whenever the result drives another
decision. Branch on `error.code` and typed evidence under `data`, never on English
diagnostics or the process status alone.

OpenSSH owns target resolution, authentication, ProxyJump and host keys. Do not
weaken its policy, store credentials in rhost, or install missing remote packages.
Do not put secrets in command text, `--env`, command files, or process arguments.

## Choose the smallest operation

- Use `exec` for ordinary foreground work. It has no default deadline; add one
  only when the operation has a meaningful bound.
- Use `session` only when an interactive shell, REPL, or debugger state must
  persist across CLI invocations. `session attach` is intentionally unavailable;
  drive state with `exec`, `send`, `read`, and `recover`.
- Use `fs` when bytes cross filesystems or a text edit needs hash-guarded atomic
  replacement. Preview destructive sync or mirror operations.
- Use `tunnel` only when an OpenSSH forward must outlive the creating invocation.
- Use `connection` and `doctor` for diagnosis. Never replay a mutation as a
  connectivity test.

A timeout, cancellation, missing completion, or output-delivery failure does not
prove a remote side effect did not happen. Inspect `data.execution` and
`data.cleanup`, then check the operation's actual remote result before retrying.

Read [references/CLI.md](references/CLI.md) for command and JSON details,
[references/RECOVERY.md](references/RECOVERY.md) after a failed call, and
[references/SAFETY.md](references/SAFETY.md) before file deletion, exposed
tunnels, privilege changes, or commands involving credentials.
