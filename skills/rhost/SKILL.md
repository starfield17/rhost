---
name: rhost
description: Run an ordinary command on an SSH-reachable remote host with local-like stdin, output, and exit behavior; use explicit job, session, file, or tunnel operations only when their extra guarantee is required.
---

# rhost

Use the command you would run locally and add only the remote execution context:

```bash
rhost exec <host> --cwd '<remote-directory>' --command '<shell program>'
```

The `--command` value runs in a fresh remote login-bash context. Pipes,
redirections, variables, and compound syntax inside that value are remote.
Pipes outside the quoted value are local:

```bash
rhost exec <host> --command 'cat result.txt | sort'   # sort is remote
rhost exec <host> --command 'cat result.txt' | sort  # sort is local
```

Direct execution needs remote `bash`, `setsid`, and a base64 decoder. Optional
features have additional dependencies; use `rhost doctor <host> --json` to
inspect them, never auto-install packages.

Human mode streams stdout/stderr and forwards stdin. It has no default execution
deadline. Add `--timeout` only when the step has a meaningful bound. Use `--json`
when the result must be classified programmatically; branch on `error.code`, and
inspect `data.exit_code`, truncation, timeout/cancellation, and
`cleanup_confirmed` rather than parsing English output.

Combine related read-only observations on the same host into one remote command
to avoid repeated SSH round trips. Keep unrelated mutations separate so each
result has an unambiguous exit and retry decision.

## Rules

- Pass exactly one non-empty shell program with `--command`. Quote it as one
  local argument; `-c` is the equivalent short form.
- Treat `--cwd` as a remote path. Quote a leading `~` so the local shell does not
  expand it.
- For every remote path argument, write `'~/path'`, not `"'~/path'"`: shell
  quotes protect the argument but are not part of the path.
- Do not place secrets in command text. It is visible to the remote process list,
  shell history, and rhost's bounded local audit summary.
- Do not weaken OpenSSH host-key or authentication policy. Diagnose failures with
  `rhost doctor <host> --json` when needed.
- Never retry a timed-out, cancelled, or disconnected side effect until
  `cleanup_confirmed` or a separate remote check establishes its state.
- Run remote search and observation with existing remote commands such as `rg`,
  `find`, `ps`, `df`, or platform tools.

## Reach for a specialized operation only when needed

| requirement | operation |
|---|---|
| command must outlive this process or connection | `rhost job` |
| interactive shell, REPL, or debugger state must persist | `rhost session` |
| bytes must cross between local and remote filesystems | `rhost fs` |
| text replacement needs a hash precondition and atomic write | `rhost fs read/write/patch` |
| port forward must survive the creating invocation | `rhost tunnel` |
| SSH or dependency diagnosis | `rhost doctor` |

Read [references/CLI.md](references/CLI.md) for those commands,
[references/RECOVERY.md](references/RECOVERY.md) after a failure, and
[references/SAFETY.md](references/SAFETY.md) before destructive file or exposed
tunnel operations.
