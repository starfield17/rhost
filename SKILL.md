# rhost — agent usage guide

`rhost` lets a coding agent (or a human) treat an SSH-reachable machine as a
reusable execution node. It orchestrates your existing OpenSSH configuration and
never duplicates SSH authentication or host-key policy.

Use `rhost` when work must run on a remote host: a GPU box, a build server, a
test environment, a Linux/WSL2 node.

**Always prefer `--json`.** Never parse human output when a JSON form exists.

---

## 0. Current capabilities

Implemented:

- `rhost hosts` — list SSH config aliases
- `rhost doctor <host>` — probe capabilities
- `rhost exec <host> -- <command...>` — stateless foreground execution
- `rhost version`

Planned (do not use yet): `session`, `job`, `fs`, `status`, `watch`, `attach`.

If a command is not listed above, it does not exist yet.

---

## 1. Discover hosts

```bash
rhost hosts --json
```

Any target you can name to `ssh` also works directly as a host argument:

- an alias from `~/.ssh/config` (e.g. `gpu`)
- `user@host` (e.g. `orangepi@192.168.123.179`)
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
  `data.stdout`, `data.stderr`, `data.timed_out`.
- On timeout the remote process group is killed, not just the local ssh.

Exit-code policy:

| status | meaning |
|---|---|
| 0–254 | remote command's own exit status |
| 255 | adapter/transport failure (see `error.code`) |
| 124 | foreground timeout (`REMOTE_COMMAND_TIMEOUT`) |

If a task will outlive a foreground timeout, do not raise the timeout
indefinitely — that is what `job` is for (once implemented).

---

## 4. Recovering from failures

Read `error.code` from the JSON envelope, never the message text:

- `SSH_UNREACHABLE` — host not reachable; retryable.
- `SSH_AUTH_FAILED` — key/agent problem; fix locally, do not retry blindly.
- `HOST_KEY_FAILED` — host key changed; investigate, never disable checking.
- `REMOTE_DEPENDENCY_MISSING` — the remote lacks something `doctor` should have
  shown.
- `REMOTE_COMMAND_TIMEOUT` — command exceeded `--timeout`.
- `CONFIG_INVALID` — bad input (e.g. an invalid env var name).

---

## 5. Authority

`rhost` can do exactly what the current OS user can do through the configured
SSH identity, and no more. It does not store private keys or passwords, does not
disable host-key verification, and does not elevate privileges.
