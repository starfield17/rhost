# rhost — agent usage guide

`rhost` lets a coding agent (or a human) treat an SSH-reachable machine as a
reusable execution node. It orchestrates your existing OpenSSH configuration and
never duplicates SSH authentication or host-key policy.

Use `rhost` when work must run on a remote host: a GPU box, a build server, a
test environment, a Linux or WSL2 node.

**Always prefer `--json`.** Never parse human output when a JSON form exists.

---

## 0. Current capabilities

Implemented:

- `rhost hosts` — list SSH config aliases
- `rhost doctor <host>` — probe capabilities
- `rhost exec <host> -- <command...>` — stateless foreground execution
- `rhost session ...` — persistent tmux-backed sessions (create/list/exec/send/read/close/attach)
- `rhost version`

Planned (do not use yet): `job`, `fs`, `status`, `watch`.

If a command is not listed above, it does not exist yet.

---

## 1. Discover hosts

```bash
rhost hosts --json
```

Any target you can name to `ssh` also works directly as a host argument:

- an alias from `~/.ssh/config` (e.g. `gpu`)
- `user@host` (e.g. `dev@build.example.internal`)
- a bare hostname or IP

---

## 2. Probe before relying on a host

```bash
rhost doctor <host> --json
```

Reports OS, kernel, arch, login shell, whether the remote state dir is writable,
and whether `bash`, `tmux`, `nohup`, `setsid`, `rsync`, `nvidia-smi`, etc. exist.
Do this once per host before expensive work. Do not assume a capability is
present.

---

## 3. Default execution: `exec`

```bash
rhost exec <host> --json --cwd ~/work/foo --timeout 60s -- pytest -q
rhost exec <host> --json --env CUDA_VISIBLE_DEVICES=0 -- python check.py
rhost exec <host> --json -- 'echo hi | wc -l'
```

Rules:

- Each call is **stateless**: no cwd, env, or shell state persists between calls.
  Set `--cwd`, `--env`, and `--timeout` explicitly.
- Command words after the host are joined with spaces and run by a login `bash`
  on the remote host, so shell syntax works. Use `--` before the command when it
  could contain rhost flags.
- The process exit status mirrors the remote command. **255 = adapter failure,
  124 = timeout.** `--json` gives the authoritative result: `data.exit_code`,
  `data.stdout`, `data.stderr`, `data.timed_out`.- On timeout the remote process group is killed, not just the local ssh.

Exit-code policy — a status in 0–254 always belongs to the **remote** command:

| status | meaning |
|---|---|
| 0–254 | remote command's own exit status |
| 124 | foreground timeout (`REMOTE_COMMAND_TIMEOUT`) |
| 255 | adapter failure: transport, validation, or usage (`error.code`) |

When a status is ambiguous (a remote command may itself exit 124 or 255), read
`--json`: `ok`, `data.exit_code`, and `error.code` are authoritative. Never
branch on the message text.

If a task will outlive a foreground timeout, do not raise the timeout
indefinitely — that is what `job` is for (once implemented).

---

## 3b. Persistent sessions

Use a session **only when state must persist** across calls: a REPL, a
debugger, an interactive CLI, or exploratory shell work where `cd`/`env` should
carry over. For ordinary commands, prefer `exec`.

```bash
rhost session create <host> --json --name debug --cwd ~/work/foo
rhost session list <host> --json
rhost session exec <host> debug --json -- 'cd src && pytest -q'
rhost session exec <host> debug --json -- pwd        # cwd persisted
rhost session send <host> debug --data 'next()\n'    # REPL / raw input
rhost session send <host> debug --key C-c            # control key
rhost session read <host> debug --json --since 0     # incremental log
rhost session close <host> debug
rhost session attach <host> debug                    # human, interactive
```

Rules:

- A session is owned by **remote tmux**, so it survives the CLI process and SSH
  disconnects. Rediscover it with `session list`; never assume it vanished.
- `session exec` persists shell state (`cd`, `export`, functions). It wraps the
  command so it produces exactly one boundary, and returns the command's real
  exit status in `data.exit_code`.
- Concurrency: multiple readers are fine, but **concurrent writers are unsafe**.
  `session exec` takes a remote lock; a human attached at the same time can race.
- `session exec` on a command that runs `exit` terminates the session and
  returns `SESSION_NOT_FOUND`. Recreate it if needed.
- `session read` returns a byte cursor: pass `data.next` back as `--since` to
  tail without re-reading. Output is ANSI-stripped.
- Agents should not use `session attach` (it is interactive); it is for humans.

---

## 4. Recovering from failures

Read `error.code` from the JSON envelope, never the message text:

- `SSH_UNREACHABLE` — host not reachable; retryable. If the message reports "no
  diagnostic", OpenSSH hid its own reason at the default log level: re-run once
  with `RHOST_SSH_LOG_LEVEL=VERBOSE` to get a real message.
- `SSH_AUTH_FAILED` — key/agent problem; fix locally, do not retry blindly.
- `HOST_KEY_FAILED` — host key changed; investigate, never disable checking.
- `REMOTE_DEPENDENCY_MISSING` — the remote lacks something `doctor` should have
  shown.
- `REMOTE_COMMAND_TIMEOUT` — command exceeded `--timeout`; the remote process
  group was killed.
- `CONFIG_INVALID` — bad input (e.g. an invalid env var name).
- `USAGE_ERROR` — the command line itself was invalid (missing host, unknown
  flag); fix the invocation, this is never retryable.
- `SESSION_NOT_FOUND` — session gone (closed, or ended by `exit` inside it);
  recreate it.
- `SESSION_UNHEALTHY` — another writer holds the session lock; retry after a
  moment. Concurrent writers are unsafe by design.
- `INTERNAL` — adapter bug; report it with the JSON envelope attached.

---

## 5. Authority

`rhost` can do exactly what the current OS user can do through the configured
SSH identity, and no more. It does not store private keys or passwords, does not
disable host-key verification, and does not elevate privileges.
