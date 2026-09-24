---
name: rhost
description: Run commands and perform explicit file, session, tunnel, or diagnostic operations on SSH-reachable hosts through an installed rhost CLI. Use when work belongs on a remote host and OpenSSH should retain authentication and host-key ownership.
---

# rhost

For first use, choose a target that already works with `ssh`, then run
`rhost doctor <host> --json` and inspect `data.capabilities` before relying
on an optional operation. `ok:true` means the probe completed; it does not
mean every capability is present. If a call fails, branch on `error.code`
and consult [references/RECOVERY.md](references/RECOVERY.md) before retrying
a mutation.
`data.capabilities.python3` reports command presence; filesystem helper
readiness is checked on the target when a file operation runs. This skill also
covers `rhost audit --json`, the local JSON Lines trail (see
[references/CLI.md](references/CLI.md)).

Use the same shell program you would run locally and add the remote context:

```bash
rhost exec <host> --cwd '<remote-directory>' --command '<shell-program>'
```

The `--command` value, or the contents of `--command-file`, is one program for a
remote login bash. Shell syntax inside that value is remote; a pipeline outside
the quoted argument is local. Simple programs can contain literal newlines;
use `--command-file ./inspect.sh` when quoting becomes awkward or a script will
be reused. Group already-known independent read-only probes in one program;
keep dependent investigation steps iterative.

## Choose output for the task

- Use the default text mode to read logs or inspect a host and decide what to
  investigate next. It streams remote stdout and stderr; it is not JSON.
- Use `--json` when the caller needs structured execution evidence or fields.
  Stdout contains one final JSON envelope, with captured command text inside it.
- Use `--json --stream` for both: live readable output goes to local stderr and
  the final JSON goes to stdout. Parse only stdout, never combined tool logs.
  Both remote streams share the live mirror; the envelope keeps them separate.

```bash
rhost exec gpu --json --stream --command 'uname -a' > result.json
jq -r '.data.output.stdout.content' result.json
```

An illustrative successful `exec` result has this shape:

```json
{
  "schema_version": 2,
  "operation": "exec",
  "ok": true,
  "host": "gpu",
  "data": {
    "execution": { "status": "completed", "exit_code": 0 },
    "cleanup": { "status": "not_attempted" },
    "output": {
      "kind": "streams",
      "stdout": { "content": "Linux\n", "bytes": 6, "truncated": false },
      "stderr": { "content": "", "bytes": 0, "truncated": false }
    },
    "duration_ms": 10
  },
  "error": null
}
```

JSON escapes newlines inside strings; decode `data.output.stdout.content` to
read the text, and check `truncated` before treating it as complete. Remote text
can guide investigation. To classify rhost failures, completion, or safe retries,
branch on `error.code` and typed evidence under `data`, never on English
diagnostics or the process status alone. This also applies during interactive
diagnosis, not only in automation.

## Targets and connections

Use an existing SSH target. For repeated use, optionally define an OpenSSH
`Host` alias such as `gpu` in the user's SSH config; `rhost hosts` lists existing
aliases, it does not register targets. See [references/CLI.md](references/CLI.md)
for an alias example and small-script patterns.

Connections normally reuse an OpenSSH ControlMaster with a 15-minute persistence
window; `--fresh` disables reuse. A new CLI or ssh process does not imply a new
SSH handshake. If repeated calls are slow, inspect `connection status` and
`doctor` without `--fresh` before attributing the delay to connection setup.

OpenSSH owns target resolution, authentication, ProxyJump and host keys. Do not
weaken its policy, store credentials in rhost, or install missing remote packages.
Do not put secrets in command text, `--env`, command files, or process arguments.

`--command`/`--command-file` text is consumed by the target account's *login*
shell, which resolves the `ssh` wrapper and then starts `bash -lc`. So the
program sees the environment that login shell already built: `~/.profile`,
`~/.bash_profile` and friends run first, exactly as an interactive login would
set them. `--fresh` means a new SSH connection with no shared ControlMaster; it
does **not** mean a clean environment or a minimal `PATH`. When a command's
result depends on which binary a name resolves to, ask the host rather than
assume: run `command -V <tool>` (and, for package ownership,
`dpkg -S`/`rpm -qf`/`brew --prefix` as that host provides) through `exec`.

Quoting ends at the local shell. A `;`, `|`, `&&` or redirection *outside* the
quoted `--command` value is consumed by your local shell before rhost sees argv,
and rhost cannot detect it — it only ever receives the one program you passed.
Put the whole program inside the quotes, and when nesting becomes awkward use
`--command-file`. That file is program text read locally and sent as one program;
it is not uploaded and no remote script is created, which covers most "temporary
remote script" needs. A real temporary file is still the caller's to create,
clean up and verify: rhost promises no cleanup of it, especially across a
disconnect.

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

If a remote dependency is missing, use the operations the host still supports.
For a host without bash, the agent's local command tool can run OpenSSH directly:
use a one-shot `ssh` command for repeatable work, or an SSH login in a local PTY
when interaction is needed. This is outside rhost's JSON and session contract;
see [references/CLI.md](references/CLI.md) for examples and
[references/SAFETY.md](references/SAFETY.md) for the limits.

A timeout, cancellation, missing completion, or output-delivery failure does not
prove a remote side effect did not happen. Inspect `data.execution` and
`data.cleanup`, then check the operation's actual remote result before retrying.

Read [references/CLI.md](references/CLI.md) for command and JSON details,
[references/RECOVERY.md](references/RECOVERY.md) after a failed call — it holds
the full `error.code` → recovery table — and
[references/SAFETY.md](references/SAFETY.md) before file deletion, exposed
tunnels, privilege changes, or commands involving credentials.

## Keep the skill and the binary in step

Updating the skill and updating the binary are separate acts, and a *symlinked*
skill drifts from the binary it points at independently. After changing either,
before trusting a call, confirm both agree:

```bash
rhost version --json              # the installed binary's version, commit and date
rhost <command> --help            # the flags this binary actually accepts
```

Treat `--help` as the authoritative flag inventory for the installed binary and
`error.code` as the branching interface; neither is frozen to this document. If
their `version` and `--help` do not match what the skill describes, install the
matching pair before continuing.
