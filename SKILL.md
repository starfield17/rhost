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
- `rhost job ...` — detached background jobs (start/list/status/logs/stop/kill)
- `rhost fs ...` — file transfer: `put`, `get` (scp) and `sync` (rsync)
- `rhost status <host>` — one read-only snapshot of system + managed state
- `rhost watch <host>` — live-refreshing monitor (human view; owns no state)
- `rhost version`

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
indefinitely — use a `job` (§3c).

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

## 3c. Durable jobs

Use a job when the work must **outlive the connection**: a training run, a long
build, a benchmark. A job is not a long `exec` and not a session: it has no
interactive shell, only a command, two log files, and an exit status.

```bash
rhost job start <host> --json --cwd ~/work/foo -- python train.py
rhost job start <host> --json --name train-1 --env CUDA_VISIBLE_DEVICES=0 -- ./train.sh
rhost job list <host> --json
rhost job status <host> <job-id> --json
rhost job logs <host> <job-id> --json --stream stdout --since 0
rhost job stop <host> <job-id> --json     # SIGTERM to the whole process group
rhost job kill <host> <job-id> --json     # SIGKILL, for jobs that ignore TERM
```

Rules:

- `job start` returns `{id, state, pid}` immediately. The id is the handle you
  keep: nothing else in the response is stable across calls. Every command above
  also accepts a `--name` while it matches exactly one job; a name that matches
  several is refused with `CONFIG_INVALID`, never guessed, so keep the id.
- The job is owned by a **detached remote process plus remote files**, so it
  survives `rhost` exiting and SSH disconnecting. Never assume a job vanished
  because the connection did: re-run `job list` to rediscover it.
- `job logs` returns a byte cursor, like `session read`: pass `data.next` back as
  `--since` to tail without re-reading. `data.more` means the stream has bytes
  beyond this chunk. `data.data` is **base64** (`data.encoding` says so) because
  log content is arbitrary bytes — decode it, never treat it as text you can
  regex in its encoded form.
- States are `starting`, `running`, `exited`, `failed`, `stopped`, `stale`.
  Branch on them, not on prose:
  - `exited` / `failed` — the job wrote its own exit code; read
    `data.exit_code` (0 means the command really succeeded).
  - `stopped` — a stop/kill was requested and the process is gone. A job
    terminated by signal records `143` (128+SIGTERM); `job kill` leaves
    `data.exit_code: -1`, because SIGKILL cannot be trapped and rhost will not
    invent a status.
  - `stale` — the pid is gone and no exit code was ever recorded (host reboot,
    an OOM kill, a stray `kill -9`). **`stale` is never a success.** Treat the
    result as unknown and re-run if the work matters.
- `job stop`/`job kill` signal the job's **process group**, so children go with
  it instead of being orphaned. Both are idempotent: requesting stop on an
  already-terminal job is not an error, and it never rewrites a recorded exit
  code. If a job ignores SIGTERM, `job stop` reports `running` after its grace
  period — that is the signal to escalate with `job kill`.
- Job state is read from remote files, so a job that has finished stays
  inspectable. There is no `job rm` yet: cleanup means deleting the remote state
  directory yourself.
- Prefer `session` when later steps depend on shell state; prefer `exec` for
  anything that finishes inside a foreground timeout.

---

## 3d. Moving files: `fs`

```bash
rhost fs put <host> ./model.py ~/work/foo/model.py --json
rhost fs get <host> ~/work/foo/results.json ./results.json --json
rhost fs sync <host> ./project ~/work/project --json --dry-run   # plan only
rhost fs sync <host> ./project ~/work/project --json             # apply
rhost fs sync <host> ./project ~/work/project --json --delete --dry-run
```

Rules:

- `put`/`get` copy **one file** (scp); `sync` copies a **directory tree**
  (rsync). Passing a directory to `put` is `CONFIG_INVALID` pointing at `sync`,
  not a silent recursive copy.
- `sync` never deletes. Only `--delete` prunes remote-only files, and it is
  refused (`SYNC_REJECTED`) when the destination is a top-level directory or a
  whole home (`/`, `/srv`, `~`, `~other`) — there is no flag that makes that
  safe, so sync into a subdirectory.
- **Read the plan before applying it.** `--dry-run` returns the same
  `data.changes` list the real sync does, with `action` one of `create`,
  `update`, `delete`, `directory`, `skip`, and the raw rsync itemize string
  beside it. An empty list means the two sides already match. `data.deletes`
  counts the destructive part.
- A destination may not contain glob characters: the remote shell would expand
  it (`SYNC_REJECTED`). Trailing slashes follow rsync's convention — `~/work/proj`
  and `~/work/proj/` mean the same thing, and rhost normalises the source.
- `data.backend` says which tool ran. A `get` into an existing directory reports
  the file it created in `data.destination`, not the directory.
- If the remote has no rsync, `sync` fails with `REMOTE_DEPENDENCY_MISSING` and
  says that `put`/`get` still work. Check with `rhost doctor <host>`.
- Transfers are **foreground and local-process-owned**: unlike a job, killing the
  CLI stops the copy. Nothing about `fs` persists.
- Errors: `TRANSFER_FAILED` (the tool's own first complaint is in `message`),
  `SYNC_REJECTED` (a refused destination), `REMOTE_DEPENDENCY_MISSING`,
  `REMOTE_COMMAND_TIMEOUT` if `--timeout` (default 5m) runs out.

---

## 3e. Host status and monitoring: `status` / `watch`

```bash
rhost status <host> --json              # one snapshot
rhost watch <host>                      # human, refreshes every 2s until Ctrl-C
rhost watch <host> --interval 5s
rhost watch <host> --json --count 1     # one envelope, for a scripted probe
```

`status` returns one read-only snapshot of the host: OS, kernel, arch, uptime,
load, CPU, memory, disk, accelerators, and the sessions and jobs rhost manages
on it. Use it to decide where to run something, or to see what is already
running, without logging in.

Rules:

- A metric the probe could not read is `null` and named in `data.unavailable`.
  **Never read a `null` metric as zero.** An empty `data.accelerators` means the
  host has no GPU (or reports none) — not a failure; a host without `nvidia-smi`,
  or WSL, still returns a full system snapshot.
- `data.online` is reachability. `data.probe_ms` is how long the snapshot took;
  it is deliberately **not** a network RTT.
- `watch` owns **no state**: every refresh re-reads the host and rediscovers its
  sessions and jobs remotely. A dropped connection shows as `data.online: false`
  with `data.offline_code`; the next successful refresh reconstructs everything
  from the host, never from local memory.
- `watch --json` prints **one envelope per refresh, one per line** (NDJSON), so
  parse it incrementally. `--count N` bounds the run; without it, `watch` runs
  until interrupted.
- Prefer `status` for a single reading; `watch` is a human live view.

## 4. Recovering from failures

Read `error.code` from the JSON envelope, never the message text:

- `SSH_UNREACHABLE` — host not reachable; retryable. If the message reports "no
  diagnostic", OpenSSH hid its own reason at the default log level: re-run once
  with `RHOST_SSH_LOG_LEVEL=VERBOSE` to get a real message.
- `SSH_AUTH_FAILED` — key/agent problem; fix locally, do not retry blindly.
- `HOST_KEY_FAILED` — host key changed; investigate, never disable checking.
- `REMOTE_DEPENDENCY_MISSING` — the remote lacks something `doctor` should have
  shown. For jobs this means `bash`, `setsid`, or `nohup` is missing.
- `REMOTE_COMMAND_TIMEOUT` — command exceeded `--timeout`; the remote process
  group was killed.
- `CONFIG_INVALID` — bad input (an invalid env var name, a job handle shaped
  like shell syntax, an ambiguous job name).
- `USAGE_ERROR` — the command line itself was invalid (missing host, unknown
  flag); fix the invocation, this is never retryable.
- `SESSION_NOT_FOUND` — session gone (closed, or ended by `exit` inside it);
  recreate it.
- `SESSION_UNHEALTHY` — another writer holds the session lock; retry after a
  moment. Concurrent writers are unsafe by design.
- `JOB_NOT_FOUND` — no job with that id (or unique name) on that host. The id
  may be from another host, or the remote state was deleted; rediscover with
  `job list`.
- `JOB_STATE_UNKNOWN` — the job exists but its state could not be read or
  written (remote state dir unwritable, helper killed mid-write). Retryable: the
  job itself is usually unaffected.
- `INTERNAL` — adapter bug; report it with the JSON envelope attached.

---

## 5. Authority

`rhost` can do exactly what the current OS user can do through the configured
SSH identity, and no more. It does not store private keys or passwords, does not
disable host-key verification, and does not elevate privileges.
