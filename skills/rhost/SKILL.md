---
name: rhost
description: Run commands and perform explicit file, session, tunnel, or diagnostic operations on SSH-reachable hosts through an installed rhost CLI. Use when work belongs on a remote host and OpenSSH should retain authentication and host-key ownership.
---

# rhost

Use a target that already works with OpenSSH: an alias, `user@example-host`, or
hostname. `rhost hosts --json` lists existing SSH aliases without contacting a
host; rhost does not register targets.

On first use of a host, run `rhost doctor <host> --json`. `ok:true` means the
probe completed: inspect `data.capabilities` and `data.state_dir_writable` for
optional operations. `python3:true` reports command presence; file operations
check helper readiness on the target themselves.

## Choose the smallest operation

| Need | Operation |
| --- | --- |
| Run an ordinary command or script | `exec` |
| Keep a shell, REPL, or debugger's state across calls | `session` |
| Copy files or make a hash-guarded text replacement | `fs` |
| Keep a port forward after this invocation exits | `tunnel` |
| Diagnose SSH reuse or dependencies | `connection`, `doctor` |
| Inspect the local operation trail | `audit` |

For work that only outlasts a tool call, background the local `rhost exec` in the
agent runtime. Work that must survive that runtime belongs to an existing remote
scheduler, invoked explicitly through `exec`. Sessions own terminal state.

## Run a program

```bash
rhost exec <host> --cwd '<remote-directory>' --command '<shell-program>'
rhost exec <host> --cwd '<remote-directory>' --command-file ./inspect.sh --json
```

The whole quoted `--command` value is one remote program; pipes, redirects and
variables outside it belong to the local shell. Literal newlines are accepted.
Use `--command-file` for awkward quoting or reusable scripts: it reads a regular
local file, sends program text without creating a remote script, and leaves
stdin free for the program. Group known independent read-only probes; keep
steps that depend on interpreting earlier output iterative.

Exec runs under remote `bash -lc`, with that shell's startup environment.
`--fresh` selects an independent SSH connection, not a clean environment.
Ordinary calls reuse an OpenSSH ControlMaster with a 15-minute persistence
window. See [CLI.md](references/CLI.md) for environment diagnosis and defaults.

## Choose output and inspect evidence

- Default text mode streams stdout/stderr for interactive investigation.
- `--json` captures separate streams in one schema-v2 envelope on stdout.
- `--json --stream` also mirrors live output to stderr. Parse only stdout,
  never combined tool logs. Exec has no default deadline; JSON capture defaults
  to 1 MiB per stream. Set `--timeout` and `--max-output-bytes` for the task.

For exec, inspect `data.execution.status` and its `exit_code` when completed;
text is under `data.output.stdout.content` and `data.output.stderr.content`.
Decode JSON strings and check each stream's `truncated` before treating it as
complete. Session exec uses merged PTY text at `data.output.content`; session
read uses `data.content` and resumes with `--since` set to `data.next`.

On failure, branch on `error.code` and operation evidence, not English messages
or process status alone. Read [RECOVERY.md](references/RECOVERY.md) before
retrying a mutation. Timeout, cancellation, missing completion and output-delivery
failure do not prove that no side effect happened. Inspect `data.execution`,
`data.cleanup` when present, and the actual remote result before resubmitting.

## Operation details and authority

Read the relevant section of [CLI.md](references/CLI.md) for commands and JSON
paths, including complete patch and batch input examples. `session attach` is
unavailable; drive a persistent program with exec/send/read/recover. File
replacement requires the hash from `fs read --json`; preview destructive sync.

OpenSSH owns authentication, host keys and ProxyJump. Never weaken its policy,
store credentials in rhost, or install missing remote dependencies. Do not put
secrets in command text, command files, `--env`, or process arguments. Read
[SAFETY.md](references/SAFETY.md) before deletion, exposed tunnels, privilege or
account changes. On minimal hosts, the direct OpenSSH fallback and its limits
are documented in CLI.md and SAFETY.md.

After installing/upgrading the binary or skill, or encountering a flag mismatch,
check `rhost version --json` and `rhost <command> --help` for the operation needed.
A symlinked skill can drift independently from the installed binary. Help is the
installed flag inventory: request it without `--json`, since `--help --json`
returns `USAGE_ERROR`. These checks are unnecessary before every command.
