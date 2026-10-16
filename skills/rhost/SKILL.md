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

## Fast decision tree

| What you want to do | Canonical invocation |
| --- | --- |
| Run an ordinary command / inspect host | `rhost exec <host> --command '<cmd>'` or `--command-file ./script.sh` |
| Zero-daemon background job | `rhost exec <host> --command 'nohup <cmd> > job.log 2>&1 </dev/null & echo $! > job.pid'` |
| Stateful shell / REPL / persistent session | `rhost session create <host> --name debug` → `exec` / `send` / `read` |
| Read remote file & get hash | `rhost fs read <host> '<remote-path>' --json` (yields `data.sha256`) |
| Write / overwrite file without local temp file | `printf '...' \| rhost fs write <host> '<remote-path>' [--if-hash <sha>] --json` or `<< 'EOF'` |
| Apply precision line edits without local temp file | `printf '{"sha256":"%s",...}' <sha> \| rhost fs patch <host> '<remote-path>' --json` or `<< 'EOF'` |
| Transfer / directory sync | `rhost fs sync <host> ./local '<remote-dir>' --dry-run --json` |
| Persistent port forwarding | `rhost tunnel open <host> --kind local --listen localhost:8080 --destination localhost:8000 --json` |
| Probe dependencies & SSH connection reuse | `rhost doctor <host> --json` / `rhost connection status <host> --json` |

## Choose the smallest operation

| Need | Operation |
| --- | --- |
| Run an ordinary command or script | `exec` |
| Keep a shell, REPL, or debugger's state across calls | `session` |
| Copy files or make a hash-guarded text replacement | `fs` |
| Keep a port forward after this invocation exits | `tunnel` |
| Diagnose SSH reuse or dependencies | `connection`, `doctor` |
| Inspect the local operation trail | `audit` |

For work that outlasts a tool call, background the local `rhost exec` in the
agent runtime, or detach a canonical zero-daemon background process remotely:
`nohup <cmd> > job.log 2>&1 </dev/null & echo $! > job.pid`. Check status via
`kill -0 $(cat job.pid) 2>/dev/null && echo RUNNING || echo STOPPED`. Work that must
survive long-term belongs to an existing remote scheduler, invoked explicitly through
`exec`. Sessions own interactive terminal state.

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

### Privileged execution (`sudo`) and environment

- `sudo` credentials do NOT persist across separate `rhost exec` calls: each call
  is an independent non-tty session, isolated by `tty_tickets`.
- `secure_path` in remote `/etc/sudoers` resets `PATH` to a minimal system path,
  stripping user additions (Homebrew, Conda, Cargo). Use absolute binary paths
  (e.g. `sudo /usr/bin/apt-get update`) or `sudo env PATH="$PATH" <cmd>`.
- For multi-step privileged tasks, run them in a single script via `--command-file`,
  a compound command under `exec`, or inside an `rhost session`.
- If `sudo` needs a password non-interactively, pass it via stdin to `sudo -S`
  (never leak passwords in command strings or audit trails). For interactive sudo,
  use direct OpenSSH with a PTY (`ssh -t <host> 'sudo ...'`).

### Canonical zero-daemon background idiom

When a task should keep running on the remote host after the CLI command completes,
avoid complex daemons. Use standard Unix process detachment:

```bash
# Launch detached process:
rhost exec <host> --cwd '~/work' --command 'nohup <command> > job.log 2>&1 </dev/null & echo $! > job.pid' --json

# Inspect liveness:
rhost exec <host> --cwd '~/work' --command 'kill -0 $(cat job.pid) 2>/dev/null && echo RUNNING || echo STOPPED' --json

# View output log or terminate:
rhost exec <host> --cwd '~/work' --command 'tail -n 50 job.log' --json
rhost exec <host> --cwd '~/work' --command 'kill $(cat job.pid) 2>/dev/null' --json
```

## File operations, Heredocs, and Stdin one-liners

Both `fs write` and `fs patch` read from stdin by default (`--from -` and `--patch -`).
Agents do not need to create local temporary files. Use piped one-liners or Heredocs directly:

```bash
# Stdin one-liner to create or write a remote file:
printf 'file contents\n' | rhost fs write <host> '<remote-path>' --parents --json

# Multi-line write via Heredoc:
rhost fs write <host> '<remote-path>' --parents --json << 'EOF'
file contents here
line 2
EOF

# Stdin one-liner hash-guarded replacement (safe compare-and-swap with hash from fs read):
printf 'updated contents\n' | rhost fs write <host> '<remote-path>' --if-hash <sha256> --json

# Multi-line replacement via Heredoc:
rhost fs write <host> '<remote-path>' --if-hash <sha256> --json << 'EOF'
updated contents
EOF

# Hash-guarded line-range patch via Heredoc:
rhost fs patch <host> '<remote-path>' --json << 'EOF'
{
  "sha256": "<sha256-from-read>",
  "edits": [{"start": 2, "end": 2, "text": "replacement line\n"}]
}
EOF
```

Creating a new file requires no hash. Replacing an existing file and every patch
require the fresh hash returned by `rhost fs read <host> '<path>' --json`.
Editing names an exact regular file and refuses symlinks in any path component.

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
