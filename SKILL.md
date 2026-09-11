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
- `rhost fs ...` — `put/get/sync/mirror/batch`, plus `read/write/patch/grep/glob`
- `rhost exec-many` — bounded parallel or sequential multi-target execution
- `rhost tunnel open/list/close` — dedicated OpenSSH-managed forwarding
- `rhost session recover` — interrupt and verify shell responsiveness
- `rhost status <host>` — one read-only snapshot of system + managed state
- `rhost watch <host>` — live-refreshing monitor (human view; owns no state)
- `rhost audit` — read the local audit log of remote operations
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
rhost exec <host> --json --cwd '~/work/foo' --timeout 60s -- pytest -q
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
- Output defaults to 1 MiB per stream. Check `stdout_truncated` and
  `stderr_truncated`; use a job for durable output. Unlimited output requires
  explicit `--max-output-bytes 0`.
- On timeout, remote cleanup is attempted. If `cleanup_confirmed` is false,
  the command may still be running; do not blindly retry side effects.

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
rhost session create <host> --json --name debug --cwd '~/work/foo'
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
rhost job start <host> --json --cwd '~/work/foo' -- python train.py
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
rhost fs put <host> ./model.py '~/work/foo/model.py' --json
rhost fs get <host> '~/work/foo/results.json' ./results.json --json
rhost fs sync <host> ./project '~/work/project' --json --dry-run   # plan only
rhost fs sync <host> ./project '~/work/project' --json             # apply
rhost fs sync <host> ./project '~/work/project' --json --delete --dry-run
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

### Reading and editing remote files

```bash
rhost fs read  <host> '~/work/project/main.go' --json   # slice + whole-file sha256
rhost fs write <host> '~/work/foo/note.txt' --from ./note.txt --if-hash <sha> --json
rhost fs patch <host> '~/work/project/main.go' --patch ./patch.json --json
rhost fs grep  <host> 'TODO' '~/work/project' --mode content --limit 100 --json
rhost fs glob  <host> '*.go' '~/work/project' --json
rhost fs mirror <host> '~/work/project' ./download --dry-run --json
rhost fs batch  <host> --manifest ./transfers.json --json
```

- These five ride an embedded Python helper on the far side, so they need remote
  `python3` (and `rg` for the two searches) and nothing else; `doctor` reports
  them, and rhost never installs anything (AGENTS.md §5).
- **Read before you write.** `fs read` returns the SHA-256 of the whole file, and
  replacing or patching an existing file without that hash is refused. A hash you
  did not just read is a guess about the file's current contents.
- A new file is created `0600` and an existing one keeps its permissions; pass
  `--mode 0755` to name them, which is how a written script becomes runnable.
- Reads are bounded (default 200 lines / 256 KiB) and searches too (default 100
  records); `truncated` is in the JSON. `next` is the offset to pass back as
  `--offset` for the following page — pagination is by *records*, not bytes.
- `fs mirror` is `sync` with the direction in the name (download); `fs batch` is
  an ordered manifest of `put`/`get` pairs. Both keep per-item results: check
  every row, because the aggregate exit code says only that *something* failed.
- Edits are UTF-8 text, limited to 8 MiB, and never applied through a symlink.

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

### 3f. Several hosts, ports, and a wedged shell

```bash
rhost exec-many --host a --host b --parallel 2 --json -- 'uname -a'
rhost exec-many --host a --host b --serial --delay 2s --stop-on-error --json -- ./deploy.sh
rhost tunnel open <host> --kind local --listen localhost:8080 --destination localhost:8000 --json
rhost tunnel open <host> --kind reverse --listen localhost:9000 --destination localhost:3000 --json
rhost tunnel open <host> --kind socks --listen localhost:1080 --json
rhost tunnel list --json
rhost tunnel close <id> --json
rhost session recover <host> <session> --json
```

- `exec-many` runs **one command on several targets** and returns one row each, in
  the order the flags were given. It is foreground orchestration, not a scheduler:
  if it must survive your laptop closing, start a `job` on each host instead.
- `--stop-on-error` stops *scheduling* new targets; a command already running on
  another host is not cancelled, and its row is not evidence that the fleet is
  clean. `--delay` is per worker, so `--serial --delay 5s` is a five-second gap
  between targets.
- Read each row's `error` and `exit_code`: the process exit code is an aggregate
  (0 all good, 1 a remote failure or a skipped target, 255 an adapter failure) and
  says nothing about which host.
- A tunnel is a **dedicated OpenSSH master with a record outside this process**: it
  keeps running after the CLI exits, and `list` is how you find ids you did not
  keep. It is not a service check — `alive` means the forward exists, not that
  anything answers on the other end; probe the port yourself.
- Forwards bind to loopback unless you pass `--allow-exposure`, which is a request
  to let other machines through: a reverse forward can put a remote service on your
  own network. The remote `sshd` has its own say, and a denied bind is reported as
  OpenSSH said it.
- `session recover` is for a session that stopped answering: it sends one
  interrupt and checks the shell responds. It does not recreate a session that has
  gone (that would silently discard the state you were trying to keep), and it can
  interrupt a program that was only slow — so it is audited.

## 4. Recovering from failures

For remote source work, prefer `fs read` and structured `fs grep/glob`. Read
before editing: use the returned SHA-256 with `fs write --if-hash` or in a patch
JSON (`sha256`, `edits` with inclusive `start/end` and replacement `text`).
`FILE_CONFLICT` requires re-reading and merging. These operations need Python 3;
search also needs rg. They never install dependencies or elevate privileges.

Use `fs mirror` for directory downloads, and `fs batch --manifest` for ordered
file pairs. `--resume` and `--checksum` opt into rsync plus end-to-end hash
verification. Check every batch result: aggregate success envelopes may contain
failed entries and a nonzero process exit status.

For `exec-many`, inspect each result's `error` and `exit_code`. Its process exit
codes are aggregate 0/1/255, unlike single-host exec. `--stop-on-error` leaves
already-started commands running to completion.

Use `session_preserved` after a timeout; `session recover` may interrupt the
active program and verifies shell responsiveness. Tunnels are persistent
OpenSSH resources: retain their ID, list to rediscover, and close when finished.
Loopback is the default; opening a listener elsewhere requires explicit intent.
A tunnel outlives the CLI process that opened it, so treat an ID you did not
print as lost: `tunnel list` is the only way back to it. Closing `rhost` never
closes a tunnel, and nothing restarts one after a reboot.

Quote remote `~/...` paths so the local shell does not expand its own home.

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
  moment. Concurrent writers are unsafe by design. `session recover` also reports
  it when a session did not answer its probe, which means the pane is wedged (a
  `sudo` prompt, an interactive program), not that the session is gone.
- `JOB_NOT_FOUND` — no job with that id (or unique name) on that host. The id
  may be from another host, or the remote state was deleted; rediscover with
  `job list`.
- `JOB_STATE_UNKNOWN` — the job exists but its state could not be read or
  written (remote state dir unwritable, helper killed mid-write). Retryable: the
  job itself is usually unaffected.
- `FILE_NOT_FOUND` — the remote path does not exist. `fs read/write/patch` take
  remote paths, which the local shell cannot stat: resolve with `fs glob` instead
  of guessing.
- `FILE_CONFLICT` — the `--if-hash` you supplied no longer describes the file, or
  the file changed while it was being read or replaced. Re-read and merge; never
  replay an old hash.
- `INVALID_TARGET` — rhost will not write through a symlink, into a directory, or
  onto anything that is not a regular file. Inspect the path; do not retry.
- `INVALID_PATCH` — the edit ranges overlap, are not 1-based inclusive, or run
  past the file. The file was not touched.
- `SEARCH_FAILED` — the remote `rg` refused the pattern or could not read the
  tree. This is the search equivalent of a command failing, not a connection
  problem.
- `INVALID_TEXT` / `FILE_TOO_LARGE` — the bytes are not something these commands
  can hold: content that is not valid UTF-8 (a binary file read with `fs read`, or
  a `fs write` of one), or a file over the 8 MiB editing limit. Use `fs put`,
  `fs get` or `fs sync` for binary and large content.
- `REMOTE_TRANSFER_FAILED` / `BATCH_FAILED` — the transfer ran and the remote side
  said no. Unlike a local `TRANSFER_FAILED`, this says nothing about your own
  machine. For `batch`, read `data.results[]`: the aggregate exit code does not
  name which entry failed.
- `TUNNEL_FAILED` / `TUNNEL_NOT_FOUND` — a forward could not be opened, or an id
  has no live record. `tunnel list` is how you rediscover ids; the id is the only
  way to close a forward, so keep it from `open`'s `data.id`.
- `INTERNAL` — adapter bug; report it with the JSON envelope attached.

---

## 5. Authority

`rhost` can do exactly what the current OS user can do through the configured
SSH identity, and no more. It does not store private keys or passwords, does not
disable host-key verification, and does not elevate privileges.

`rhost` keeps a local audit log of the remote operations it runs (exec, sessions,
jobs, transfers, status) at `~/.local/state/rhost/audit.jsonl` — operations, not
secrets: no environment maps, and `session send` records the key but never the
injected data. Read it back with `rhost audit --json` (most recent 20 by default;
`--limit 0` for all, `--host H` to filter). Set `RHOST_AUDIT=0` to turn it off.
Auditing is fail-open: a write failure never blocks a remote operation.

What gets a line is an operation that *did* something: writes and transfers
(`fs put/get/sync/mirror/batch`, each entry of a batch under its own operation),
`fs write` and `fs patch`, session create/exec/recover/close, job start/stop/kill,
`tunnel open` and `tunnel close` — including the ones that failed, since a refused
remote write is exactly the history you want later. Reads are not: `fs read`,
`fs grep`, `fs glob`, `status`, `doctor`, `hosts` and `audit` are polling, and
recording them would turn the trail into a log of how often something asked.
A `tunnel open` line carries the kind, the bind address and the destination, so an
exposed forward is auditable after the fact.
