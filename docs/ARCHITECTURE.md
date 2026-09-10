# Remote Host Adapter — Architecture & Implementation Framework

> Working name: **rhost**  
> Audience: Codex implementing the repository  
> Language recommendation: **Go**  
> Architecture style: disposable CLI + externalized persistence  
> Required product shape: **Skill + source + release binary**, not MCP-first

---

## 0. Implementation directive

Build a general remote-host adapter, not a YOLO tool and not an SSH wrapper collection.

The core model is:

```text
RemoteHost
├── Exec       stateless foreground command
├── Session    persistent interactive terminal
├── Job        durable background process
├── FS         file transfer / synchronization
└── Status     generic host telemetry and managed-state discovery
```

The most important invariant is:

> **Anything promised to survive a CLI invocation must be owned outside the CLI process.**

The CLI may exit after every command.

Do not solve persistence by keeping a hidden `rhost` server alive in v0.1.

---

# Part I — System shape

## 1. Top-level architecture

```text
                         Coding Agent
                              │
                         reads SKILL.md
                              │
                    shell command invocation
                              ▼
                     ┌─────────────────┐
                     │   rhost CLI     │
                     │  disposable     │
                     └────────┬────────┘
                              │
                 ┌────────────┼─────────────┐
                 │            │             │
                 ▼            ▼             ▼
          human formatting   JSON       watch/TUI
                 │            │             │
                 └────────────┼─────────────┘
                              ▼
                     ┌─────────────────┐
                     │ application     │
                     │ use-cases       │
                     └────────┬────────┘
                              │
          ┌───────────────────┼────────────────────┐
          ▼                   ▼                    ▼
     transport            persistence          telemetry
     OpenSSH              adapters             probes
          │                   │                    │
          ├──────────────┬────┴─────────┬──────────┤
          ▼              ▼              ▼          ▼
       SSH exec       remote tmux   remote jobs   remote FS
          │              │              │          │
          └──────────────┴───────┬──────┴──────────┘
                                 ▼
                          Remote Linux host
```

No project-specific logic belongs below the CLI.

---

## 2. Persistence ownership

This table is an architectural contract.

| Concern | v0.1 owner | Why |
|---|---|---|
| SSH connection reuse | local OpenSSH ControlMaster | survives `rhost` process exit |
| stateless exec | fresh SSH channel/process | predictable semantics |
| persistent shell | remote tmux | survives local process/network disconnect |
| session terminal log | remote state file | allows incremental reads after reconnect |
| background job | remote process/process group | survives SSH disconnect |
| job metadata/logs | remote state directory | rediscoverable |
| host aliases/auth | `~/.ssh/config` | do not duplicate SSH |
| adapter config | local `rhost` config | project-specific defaults do not belong here |
| UI/watch state | reconstructed | monitor must not own runtime state |

Do not add in-memory ownership where a process restart would violate this table.

---

# Part II — Repository boundaries

## 3. Recommended Go repository layout

Use Go-native layout rather than an artificial `src/` layer:

```text
rhost/
├── SKILL.md
├── README.md
├── docs/
│   ├── PROJECT_OVERVIEW.md
│   └── ARCHITECTURE.md
├── go.mod
├── go.sum
│
├── cmd/
│   └── rhost/
│       └── main.go
│
├── internal/
│   ├── app/
│   │   ├── exec.go
│   │   ├── session.go
│   │   ├── job.go
│   │   ├── fs.go
│   │   ├── status.go
│   │   └── doctor.go
│   │
│   ├── transport/
│   │   └── openssh/
│   │       ├── client.go
│   │       ├── controlmaster.go
│   │       ├── command.go
│   │       └── config.go
│   │
│   ├── session/
│   │   └── tmux/
│   │       ├── manager.go
│   │       ├── protocol.go
│   │       ├── log.go
│   │       └── attach.go
│   │
│   ├── job/
│   │   └── detached/
│   │       ├── manager.go
│   │       ├── wrapper.go
│   │       ├── metadata.go
│   │       └── logs.go
│   │
│   ├── fileops/
│   │   ├── transfer.go
│   │   └── sync.go
│   │
│   ├── telemetry/
│   │   ├── linux.go
│   │   ├── nvidia.go
│   │   └── model.go
│   │
│   ├── host/
│   │   ├── registry.go
│   │   └── capabilities.go
│   │
│   ├── config/
│   │   ├── config.go
│   │   └── paths.go
│   │
│   ├── output/
│   │   ├── json.go
│   │   └── human.go
│   │
│   └── watch/
│       └── watch.go
│
├── schemas/
│   └── result-v1.schema.json
│
├── scripts/
│   └── install.sh
│
└── .github/
    └── workflows/
        └── release.yml
```

### Boundary rule

`cmd/rhost` wires dependencies and parses CLI arguments. It contains no remote-control logic.

`internal/app` owns use-case semantics. It depends on interfaces/capabilities, not on terminal formatting.

Backend packages implement mechanisms:

- `transport/openssh`
- `session/tmux`
- `job/detached`
- `fileops`
- `telemetry`

`output` only formats models already produced by the application layer.

Use Go `internal/` aggressively. Do not create a generic `utils/` package.

---

## 4. Suggested core interfaces

Do not over-abstract before the first implementation exists. These interfaces are enough to separate semantics from mechanisms.

Conceptually:

```go
type ExecRequest struct {
    Host    string
    Command string
    Cwd     string
    Env     map[string]string
    Timeout time.Duration
}

type ExecResult struct {
    Host       string
    ExitCode   int
    Stdout     []byte
    Stderr     []byte
    Duration   time.Duration
    TimedOut   bool
}

type Executor interface {
    Exec(ctx context.Context, req ExecRequest) (ExecResult, error)
}
```

Session:

```go
type SessionManager interface {
    Create(ctx context.Context, req CreateSessionRequest) (Session, error)
    List(ctx context.Context, host string) ([]Session, error)
    Exec(ctx context.Context, req SessionExecRequest) (SessionExecResult, error)
    Send(ctx context.Context, req SessionSendRequest) error
    Read(ctx context.Context, req SessionReadRequest) (SessionReadResult, error)
    Close(ctx context.Context, host, id string) error
}
```

Job:

```go
type JobManager interface {
    Start(ctx context.Context, req StartJobRequest) (Job, error)
    List(ctx context.Context, host string) ([]Job, error)
    Status(ctx context.Context, host, id string) (JobStatus, error)
    Logs(ctx context.Context, req JobLogsRequest) (JobLogChunk, error)
    Signal(ctx context.Context, host, id string, sig string) error
}
```

Keep these internal until a second frontend genuinely needs a public Go library.

---

# Part III — Host/configuration model

## 5. OpenSSH config is the authentication source of truth

Do not create a parallel SSH credential format.

Expected user configuration:

```sshconfig
Host gpu
    HostName gpu.example.internal
    User dev
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 30
    ServerAliveCountMax 3
```

Use placeholder names in documentation. Real addresses, accounts, and personal
SSH aliases must not appear in tracked files (`AGENTS.md` §1).

`rhost` should invoke the user's OpenSSH client with `Host=gpu` semantics so that it naturally inherits:

- `HostName`;
- `User`;
- `Port`;
- `IdentityFile`;
- ssh-agent;
- `ProxyJump`;
- `Include`;
- `known_hosts`;
- host key policy.

### Adapter config

Use a separate local config only for rhost-specific settings.

Example:

```toml
[hosts.gpu]
ssh = "gpu"
default_shell = "bash"
remote_state_dir = "~/.local/state/rhost"
tags = ["gpu", "wsl2"]

[defaults]
control_persist = "15m"
foreground_timeout = "60s"
watch_interval = "2s"
```

Do not put private keys or passwords here.

---

## 6. Host capability discovery

`rhost doctor <host>` should probe and return capabilities rather than silently assuming them.

Minimum probes:

```text
SSH reachable
batch authentication works
bash available
tmux available
nohup available
setsid available
ps available
remote state dir writable
rsync available locally
rsync available remotely
nvidia-smi available
systemd --user available
WSL detected or not
```

Example human output:

```text
gpu
SSH                 OK
Host key            verified by OpenSSH
bash                OK
tmux                OK
durable jobs        OK (nohup + setsid)
rsync               OK
NVIDIA telemetry    OK
WSL2                yes
```

Example JSON should expose the same facts mechanically.

Do not auto-install missing packages in v0.1.

---

# Part IV — SSH transport

## 7. v0.1 transport: system OpenSSH

The first implementation should intentionally use the system `ssh` binary.

Reasons:

- exact compatibility with existing `~/.ssh/config`;
- uses the operator's ssh-agent and keychain behavior;
- mature host-key verification;
- ProxyJump and uncommon SSH options work without reimplementation;
- ControlMaster provides connection persistence outside the CLI process;
- simplest path to a reliable first release on the primary client platform.

The single release binary therefore orchestrates OpenSSH; it does not need to reimplement the SSH protocol in v0.1.

### Later option

A native Go transport using `golang.org/x/crypto/ssh` may be added if there is a concrete need for:

- no external OpenSSH dependency;
- Windows-native local support;
- tighter streaming control;
- a future daemon;
- multiplexing not expressible cleanly through OpenSSH.

Do not build both backends in the first release.

---

## 8. ControlMaster management

`rhost` should maintain its own socket namespace, for example:

```text
~/.cache/rhost/ssh/
    <hash>.sock
```

Use a hashed path because Unix-domain socket paths have length limits.

That limit is a hard budget, not a style preference: `sun_path` allows 104 bytes
including the NUL on BSD-derived systems (108 on Linux), and OpenSSH substitutes a
fixed 40-character digest for `%C`. A deep cache root therefore breaks *every*
command with `ControlPath too long` - reachable through `RHOST_CACHE_DIR` or
`$XDG_CACHE_HOME`, which is how this was first hit. `config.ControlDir()` checks
the expanded length and falls back to a short per-user root (`/tmp/rhost-<uid>/ssh`)
when the cache root is too deep; it must not use `os.TempDir()` for that fallback,
because on macOS the per-session temp dir is itself long. The directory rhost
creates and the directory it binds in must come from the same function.

Conceptual options:

```text
ControlMaster=auto
ControlPersist=15m
ControlPath=~/.cache/rhost/ssh/%C
BatchMode=yes
```

Operations:

```text
ensure master exists
check master
open command channel
allow ControlPersist to keep transport alive
explicit close only when requested / cleanup requires it
```

The transport lifetime is not the CLI lifetime.

### Required test

Start one master, run multiple `rhost exec` invocations as separate OS processes, and prove they reuse the same master.

Do not infer this from timing alone. Use an observable OpenSSH control check or debug evidence in integration tests.

---

## 9. Command construction

Avoid accidental dependence on the remote account's default shell.

For normal Linux execution use a controlled shell, initially:

```text
bash -lc <command>
```

`cwd` and environment must be encoded explicitly.

Conceptually:

```bash
cd -- "$cwd"
export KEY=...
exec-or-run command
```

Do not rely on a previous `cd` or `export`.

All shell quoting/escaping must live in one module with tests.

Never build remote commands by naive string concatenation across the codebase.

---

# Part V — `exec`

## 10. CLI surface

Target shape:

```bash
rhost exec <host> [flags] -- <command...>
```

Examples:

```bash
rhost exec gpu -- pwd

rhost exec gpu \
  --cwd ~/work/foo \
  --timeout 30s \
  -- pytest -q

rhost exec gpu \
  --env CUDA_VISIBLE_DEVICES=0 \
  --json \
  -- python scripts/check.py
```

### Semantics

Each call gets a fresh remote execution context.

Returned model:

```json
{
  "schema_version": 1,
  "operation": "exec",
  "ok": false,
  "host": "gpu",
  "data": {
    "exit_code": 1,
    "stdout": "...",
    "stderr": "...",
    "timed_out": false,
    "duration_ms": 831
  },
  "error": null
}
```

`ok` means the adapter operation completed as designed. The remote command may still have a non-zero `exit_code`.

For shell scripting, the CLI process should normally mirror the remote command's exit status when execution reached the remote process. Infrastructure/adapter failures should use a documented separate convention.

The JSON document remains the authoritative diagnosis.

---

## 11. Foreground timeout

Foreground operations must have an upper bound.

A reasonable default may exist for humans, but the Skill should teach agents to set explicit timeouts for uncertain operations.

When a task is expected to outlive a foreground timeout, the correct action is:

```text
use `job`, not a larger and larger `exec` timeout
```

Cancellation must attempt to terminate the remote foreground process, not only kill the local `ssh` subprocess.

Design and test this explicitly; OpenSSH process termination alone is not sufficient evidence that the remote child died.

---

# Part VI — `session`

## 12. Why tmux is the v0.1 session backend

A persistent session must survive:

- one `rhost` invocation ending;
- the agent process ending;
- SSH disconnect;
- temporary network loss;
- a later `rhost` invocation.

Remote tmux already owns exactly this lifetime.

Therefore:

```text
rhost CLI
   │
   ├─ create/list/send/read/attach/close
   │
   ▼
SSH
   │
   ▼
remote tmux session
   │
   ▼
managed shell / REPL / debugger
```

Do not store a live PTY object only in local memory.

---

## 13. Session identity and remote layout

Namespace all managed sessions.

Example tmux session name:

```text
rhost_s_<short-id>
```

Remote metadata:

```text
~/.local/state/rhost/
└── sessions/
    └── s_01J.../
        ├── meta.json
        └── pty.log
```

`meta.json` example:

```json
{
  "schema_version": 1,
  "id": "s_01J...",
  "name": "debug",
  "tmux_session": "rhost_s_01J...",
  "created_at": "2026-09-10T12:00:00Z",
  "created_by": "rhost",
  "initial_cwd": "/home/dev/work/foo",
  "shell": "bash"
}
```

The remote state is authoritative for discovery.

If metadata exists but tmux does not, report the session as stale/dead. Do not pretend it is alive.

---

## 14. Session creation

Prefer launching a known shell explicitly, for example:

```text
bash --noprofile --norc -i
```

or a configurable login-shell mode when needed.

Do not blindly inherit fish/zsh/bash differences and then make the command protocol guess them.

The session backend should set an adequate tmux history limit.

Attach a pane output log using tmux `pipe-pane` or an equivalent mechanism so output can be read incrementally after reconnect.

The exact mechanism must be integration-tested from a real client host against a
real remote Linux host.

---

## 15. Session command modes

A session needs **two** kinds of input.

### `session exec`

For a normal shell command where the caller wants:

```text
command
→ output
→ exit code
→ command finished
```

Example:

```bash
rhost session exec gpu debug -- 'cd ~/work/foo'
```

This path should use a command-boundary protocol.

### `session send`

For raw interactive input where no shell command boundary is expected:

```bash
rhost session send gpu debug --data 'next\n'
rhost session send gpu debug --key C-c
```

Use this for:

- Python REPL;
- gdb/lldb/pdb;
- interactive installers;
- TUI tools;
- Ctrl-C and other control keys.

Do not force `session exec` semantics onto arbitrary interactive programs.

---

## 16. Reliable input injection

Do not implement arbitrary commands by embedding them directly in:

```text
tmux send-keys "...user text..."
```

That creates quoting and special-key problems.

Prefer:

```text
local command bytes
    ↓
encode/transfer safely
    ↓
tmux load-buffer
    ↓
tmux paste-buffer
    ↓
send Enter separately
```

or an equivalent binary-safe path.

Raw keys such as `C-c` should use explicit tmux key operations.

Before a managed `session exec`, the backend may need to normalize the shell input line (for example Ctrl-C/Ctrl-U) only when it is known to be at a managed shell prompt. Do not send destructive control sequences blindly while an interactive program owns the foreground.

---

## 17. Command-boundary protocol

This is one of the places where Codex should inspect the local `portal-mcp-server` source.

Portal's persistent-shell design is useful reference material for:

- marking command completion in a PTY stream;
- recovering the remote exit code;
- avoiding prompt-string guessing;
- soft-cancel behavior;
- resynchronizing after an interactive prompt;
- serializing access to one shell.

For `rhost`, adapt the idea to a tmux-owned session.

Two acceptable approaches:

### Preferred if verified through tmux

Use an OSC 133-style completion marker carrying the exit status.

### Simpler fallback

Use a cryptographically random per-command nonce:

```text
__RHOST_DONE_<128-bit-random>__:<exit-code>
```

The marker is written only after the command returns.

The parser reads the session log until it observes that exact nonce.

Do not use a fixed sentinel such as `DONE`.

Whichever protocol is selected, add an integration test proving:

- normal output may contain arbitrary similar strings;
- UTF-8 boundaries do not corrupt parsing;
- non-zero exit codes are returned;
- timeout/cancel leaves the shell in a known state or marks the session unhealthy;
- separate CLI processes can continue the same session.

---

## 18. Incremental session reads

Target:

```bash
rhost session read gpu debug --since 18422 --json
```

Result:

```json
{
  "session_id": "s_01J...",
  "from": 18422,
  "next": 19284,
  "data": "...",
  "more": false
}
```

Use a byte offset or another explicit cursor.

Do not make the agent repeatedly parse a full `tmux capture-pane` snapshot.

A full-screen capture command may still exist for humans/debugging, but incremental log reading should be the machine-facing primitive.

---

## 19. Human attach

A human should be able to enter the exact same remote terminal:

```bash
rhost session attach gpu debug
```

Implementation can simply exec an interactive SSH command equivalent to:

```text
ssh -t <host> tmux attach-session -t <managed-session>
```

This is a core reason to keep sessions remote and tmux-backed.

Human attach and agent interaction must target the same tmux pane/session.

Concurrency policy for simultaneous human and agent input should initially be simple and explicit:

- multiple readers are fine;
- concurrent writers are unsafe;
- `session exec` must hold a logical session lock;
- human attach should warn that automated writes can race;
- sophisticated ownership/takeover modes may be added later.

Do not build a collaborative terminal protocol in v0.1.

---

# Part VII — `job`

## 20. Job semantics

Target:

```bash
rhost job start <host> [flags] -- <command...>
```

Example:

```bash
rhost job start gpu \
  --cwd ~/work/foo \
  -- python train.py
```

Result:

```json
{
  "job_id": "j_01J...",
  "state": "running",
  "pid": 18372
}
```

The command continues after:

- `rhost` exits;
- the starting terminal closes;
- SSH disconnects.

It does not promise survival of a remote OS shutdown or WSL shutdown.

---

## 21. Remote job state layout

Use persistent per-user state, not `/tmp`, unless there is a strong reason otherwise.

Example:

```text
~/.local/state/rhost/
└── jobs/
    └── j_01J.../
        ├── meta.json
        ├── command.sh
        ├── stdout.log
        ├── stderr.log
        ├── pid
        ├── pgid
        ├── exit_code
        └── finished_at
```

Permissions should default to user-private.

Example metadata:

```json
{
  "schema_version": 1,
  "id": "j_01J...",
  "cwd": "/home/dev/work/foo",
  "command": "python train.py",
  "started_at": "2026-09-10T12:00:00Z",
  "backend": "detached"
}
```

Do not store secrets in command metadata if they were passed through environment or future secret mechanisms.

---

## 22. Detached process backend

v0.1 may use:

```text
nohup + setsid + shell wrapper
```

The wrapper should:

1. establish cwd/env;
2. launch the command in its own process group/session;
3. redirect stdout and stderr to distinct files;
4. record PID/PGID;
5. atomically write exit code on completion;
6. atomically write completion time;
7. preserve enough metadata for later rediscovery.

Prefer transferring or generating a small wrapper script over constructing an unreadable deeply quoted one-liner.

### Later backend

If real usage proves a need, add a `systemd --user` job backend.

Do not require systemd for the first usable release.

---

## 23. Job status

`rhost job status` must not infer running state from log freshness.

Use process existence and terminal metadata.

States:

```text
starting
running
exited
failed
stopped
unknown
stale
```

The exact state machine should be documented in code and tests.

If the job metadata says it was running but the PID no longer exists and there is no exit-code file, report `unknown` or `stale`, not `success`.

---

## 24. Job log polling

Support offsets:

```bash
rhost job logs gpu j_01J... --stream stdout --since 8192 --json
```

Return:

```json
{
  "job_id": "j_01J...",
  "stream": "stdout",
  "from": 8192,
  "next": 12288,
  "data": "...",
  "more": false
}
```

This makes polling cheap for agents.

If arbitrary binary output must be supported, expose encoding metadata or base64 when needed.

Codex should inspect Portal's background-job polling implementation for ideas about:

- offset-based incremental reads;
- bounded chunks;
- process discovery after reconnect;
- job metadata lifecycle.

Do not copy its local in-memory registry as the source of truth.

---

## 25. Job signals

Minimum:

```bash
rhost job stop <host> <id>
rhost job kill <host> <id>
```

Prefer signaling the process group rather than one shell PID so children do not remain orphaned.

Behavior should be observable and idempotent.

A stop request against an already terminal job should not invent an error if the requested final condition already holds.

---

# Part VIII — files

## 26. File operations

Target surface:

```text
rhost fs put
rhost fs get
rhost fs sync
```

Examples:

```bash
rhost fs put gpu ./model.py ~/work/foo/model.py
rhost fs get gpu ~/work/foo/results.json ./results.json
rhost fs sync gpu ./project ~/work/project
```

`fs` operates on generic paths. It knows nothing about repositories, datasets, checkpoints, or YOLO.

---

## 27. v0.1 transfer backend

A pragmatic first release may use:

- `rsync` for directory synchronization when available;
- `scp` or rsync for simple file transfer.

Advantages:

- mature behavior;
- incremental sync;
- proven SSH integration;
- uses the same OpenSSH configuration.

Later, native SFTP can remove dependencies or provide tighter structured progress.

Codex may study Portal's SFTP/file-transfer code for design ideas, especially structured errors and transfer integrity, but native SFTP is not required to validate the overall architecture.

---

## 28. Safe synchronization

`sync` is the operation most likely to cause accidental data loss.

Requirements:

- no implicit deletion by default;
- `--delete` must be explicit;
- provide `--dry-run`;
- print/return the effective source and destination;
- reject obviously dangerous empty/root destinations where possible;
- keep agent JSON and human preview consistent;
- do not silently follow unexpected symlinks across boundaries.

Recommended agent workflow:

```text
dry run
→ inspect plan
→ execute
```

The Skill should teach this for destructive sync modes.

---

# Part IX — status and monitor

## 29. `status`

`rhost status <host>` returns one snapshot.

Generic Linux fields:

```text
hostname
OS / kernel / WSL detection
uptime
load
CPU count / utilization
memory
disk
network reachability / RTT
managed sessions
managed jobs
accelerators
```

Accelerators are an extensible list:

```json
{
  "accelerators": [
    {
      "vendor": "nvidia",
      "type": "gpu",
      "name": "NVIDIA GeForce RTX 4060",
      "utilization_percent": 93,
      "memory_used_bytes": 6657199308,
      "memory_total_bytes": 8589934592,
      "temperature_c": 66
    }
  ]
}
```

If `nvidia-smi` is missing or a field is unsupported in WSL, represent it as unavailable. Do not fail the entire host snapshot.

---

## 30. Remote probe design

Avoid one SSH process per metric.

A single `status` snapshot should execute one bounded remote probe that gathers the necessary fields, preferably returning a simple machine-readable intermediate form.

The probe may call:

```text
/proc
uname
df
ps
tmux
nvidia-smi
```

Keep it read-only.

The probe should be versioned so parser expectations are explicit.

Do not install a remote daemon in v0.1 just to collect telemetry.

---

## 31. `watch`

`rhost watch <host>` is a human-facing live monitor.

It repeatedly calls the same application-layer snapshot logic at a configurable interval, usually around 2 seconds.

It must not become a separate state owner.

```text
watch
  │
  ├─ fetch status
  ├─ render
  ├─ sleep
  └─ repeat
```

If network connectivity disappears:

```text
online → reconnecting/offline
```

When connectivity returns, the monitor reconstructs the host state.

It should then rediscover tmux sessions and remote jobs instead of assuming continuity from local memory.

v0.1 can be a terminal UI. A browser dashboard is not required.

---

# Part X — structured output

## 32. JSON is a public compatibility surface

Agent-facing output is part of the product API.

Use a schema version from day one.

Common envelope:

```json
{
  "schema_version": 1,
  "operation": "status",
  "ok": true,
  "host": "gpu",
  "data": {},
  "error": null
}
```

Error example:

```json
{
  "schema_version": 1,
  "operation": "exec",
  "ok": false,
  "host": "gpu",
  "data": null,
  "error": {
    "code": "SSH_UNREACHABLE",
    "message": "host did not accept an SSH connection",
    "retryable": true
  }
}
```

Do not put ANSI color or progress spinners on stdout in JSON mode.

Human progress may go to stderr.

---

## 33. Error taxonomy

Start small and stable.

Candidate codes:

```text
USAGE_ERROR
CONFIG_INVALID
HOST_UNKNOWN
SSH_UNREACHABLE
SSH_AUTH_FAILED
HOST_KEY_FAILED
REMOTE_DEPENDENCY_MISSING
REMOTE_COMMAND_TIMEOUT
SESSION_NOT_FOUND
SESSION_UNHEALTHY
JOB_NOT_FOUND
JOB_STATE_UNKNOWN
TRANSFER_FAILED
SYNC_REJECTED
UNSUPPORTED_REMOTE_OS
```

Keep low-level OpenSSH stderr available for diagnosis but do not force the agent to classify behavior by matching English error strings.

`USAGE_ERROR` is reserved for argument/flag parsing failures, which happen before any command runs. The usage path still emits a normal envelope when `--json` is requested, because every agent-visible outcome needs a machine-readable form.

OpenSSH runs at `LogLevel=ERROR` by default so that successful commands keep stderr clean. That level also hides ssh's own reason for some connection failures; `RHOST_SSH_LOG_LEVEL` raises it (for example `VERBOSE`) without rebuilding the binary, and the fallback message points there.

---

# Part XI — SKILL.md

## 34. Required Skill behavior

The repository should ship a concise `SKILL.md`.

Its main rules should be operational facts, not general software-engineering advice.

Suggested behavior:

```text
Use rhost when work must execute on an SSH-configured remote host.

Before first use on a host:
  rhost doctor <host> --json

Default:
  rhost exec ... --json

Use session only when:
  cwd/env/interactive terminal state must persist.

Use job when:
  the task must survive disconnect,
  or the expected runtime is longer than a foreground operation.

For files:
  use rhost fs ...
  use dry-run before sync deletion.

For machine state:
  use rhost status --json.
  `rhost watch` is for humans, not for agent parsing.

After a disconnect:
  rediscover session/job state; do not assume it vanished.

Never parse the human TUI when a JSON form exists.
```

The Skill should name actual CLI commands and behaviors.

It should not contain a tutorial on SSH, tmux, Go, or system design.

---

# Part XII — security and audit

## 35. Security stance

`rhost` is a remote execution adapter, not a sandbox.

Its authority equals the configured SSH identity.

Hard requirements:

- never disable host-key checking globally;
- never persist private keys;
- never persist passwords;
- default agent use to non-interactive authentication;
- do not silently elevate privileges;
- do not auto-install remote dependencies;
- keep managed remote state user-private;
- redact known secret-bearing environment values from logs/metadata when such a mechanism is added.

Do not claim that a command is safe merely because it ran through `rhost`.

---

## 36. Audit trail

A lightweight local audit log is useful:

```text
~/.local/state/rhost/audit.jsonl
```

Record operations, not secrets:

```json
{
  "time": "...",
  "host": "gpu",
  "operation": "exec",
  "cwd": "/home/dev/work/foo",
  "command_summary": "pytest -q",
  "exit_code": 1,
  "duration_ms": 831
}
```

Do not record full environment maps by default.

Do not make audit logging a prerequisite for v0.1 execution if the design would make a full disk brick all remote work; choose and document fail-open/fail-closed behavior consciously.

---

# Part XIII — Portal source as reference

## 37. What Codex should inspect in local `portal-mcp-server`

The user will place the Portal source locally.

Use it as a source of implementation ideas.

In current Portal versions, useful areas are conceptually equivalent to:

```text
connection manager
session manager
remote shell engine
job manager
file operations / SFTP
security / audit
```

If the local checkout contains files such as:

```text
connection_manager.py
session_manager.py
remote_bash.py
job_manager.py
file_ops.py
security.py
audit.py
```

inspect them.

### Borrow the ideas

Especially study:

1. SSH transport reuse and channel reuse.
2. The semantic split between stateless exec and persistent shell.
3. PTY command-completion markers and exit-code recovery.
4. Shell locking / avoiding concurrent readers on one PTY.
5. Timeout, Ctrl-C soft cancellation, and resynchronization.
6. Detached background job launch.
7. Incremental job-log reads by offset.
8. Structured result/error models.
9. Transfer behavior and bounded outputs.
10. Separation of execution logic from security/audit logic.

### Do not copy the product shape

Do not port:

- FastMCP tool registration as the primary frontend;
- mandatory MCP transport;
- a long-running Python server as the owner of persistence;
- process-memory-only session registries;
- Python/asyncio-specific lifecycle assumptions.

The important difference is:

```text
Portal-style server:
server process owns SSH objects / persistent PTY

rhost v0.1:
CLI owns nothing durable
ControlMaster owns connection reuse
remote tmux owns session
remote process/state dir owns job
```

If Portal has a better implementation detail that fits this ownership model, use it.

---

# Part XIV — releases and installation

## 38. Release artifacts

Automate releases with GoReleaser or a small GitHub Actions matrix.

Initial artifacts:

```text
darwin/arm64
darwin/amd64
linux/amd64
linux/arm64
```

Priority:

```text
1. darwin/arm64
2. linux/amd64
3. linux/arm64
4. darwin/amd64
```

The project should be runnable after copying one binary into `$PATH`.

The installer may download the appropriate release.

Do not require Python, Node, or a virtual environment.

---

## 39. Version output

Provide:

```bash
rhost version
```

Include:

```text
version
git commit
build date
Go version
schema version
```

This is important when an Agent reports a remote-control behavior that may depend on CLI version.

---

# Part XV — testing

## 40. Test layers

### Unit tests

Cover:

- shell quoting;
- OpenSSH argument construction;
- JSON schemas;
- status parsing;
- tmux command construction;
- session marker parser;
- byte-offset log cursors;
- job state machine;
- metadata serialization;
- dangerous sync-path rejection.

### Local integration tests

Use a disposable local SSH server/container when practical to test:

- OpenSSH invocation;
- ControlMaster reuse;
- stdout/stderr separation;
- non-zero exit;
- timeout;
- upload/download.

### Live remote integration tests

The critical features require a real remote Linux host.

Gate them behind explicit configuration, for example:

```text
RHOST_TEST_LIVE=1
RHOST_TEST_HOST=gpu
```

Never make ordinary unit tests unexpectedly touch a real machine.

Live tests must drive the **built binary as a child process**, one process per
step, never the Go API in-process. An in-process test cannot distinguish
"rhost persisted this" from "rhost happened to keep it in a map until now", and
that distinction is the product (AGENTS.md §4).

| suite | command |
|---|---|
| exec, timeout, doctor, transport reuse | `make test-live` |
| session persistence | `make test-live-session` |
| everything | `make test-live-all` |

---

## 41. Required persistence tests

These are product-defining.

### Test A — connection persistence

```text
process 1: rhost exec
process exits

process 2: rhost exec
process exits

prove both reused the same ControlMaster
```

Automated: `TestLiveTransportReuse`. It reads the master pid back out of OpenSSH
(`ssh -O check`) before and after the second process, so equal pids are positive
evidence of reuse rather than an absence of errors.

### Test B — session persistence

```text
rhost session create
rhost process exits

new process:
session exec "cd ..."
process exits

new process:
session exec "pwd"
prove cwd persisted
```

Then kill the local invoking process abruptly and repeat discovery.

Automated: `TestLiveSession`, one CLI process per step: `create`, `cd`, then
`pwd` in a later process; `export` then `echo $VAR`; a stable `$$` proving the
same pane; and finally `SIGKILL` of a mid-command client, after which the session
is still listed, the still-running remote command holds the writer lock
(`SESSION_UNHEALTHY`, retryable), and the session becomes usable again on its
own.

### Test C — job persistence

```text
rhost job start "sleep ...; emit output"
starting process exits
close SSH master if desired

later:
rhost job status
rhost job logs
prove job survived and is discoverable
```

Not automated yet: blocked on Milestone 3.

### Test D — network interruption

Start a session and a job, temporarily break the local connection, reconnect, and prove:

- session is rediscovered;
- job is rediscovered;
- monitor reconstructs state.

Manual only. Deliberately not automated: nothing in this repository should imply
that a script can cut a real network link on someone else's host, and a simulated
disconnection would not prove what the test claims to prove.

---

## 42. Prove boundary checks work

Use Go language boundaries first.

`internal/` should prevent backend internals leaking across modules.

If an additional dependency rule is added, intentionally violate it once and confirm the check fails.

A boundary rule that has never failed in a test is not trusted.

This principle is taken from the useful pattern in the provided repository-shaping Skill: machine-enforced boundaries are more valuable than prose-only rules.

---

# Part XVI — staged implementation

## 43. Milestone 0 — repository skeleton

Deliver:

```text
SKILL.md
README
docs
Go module
CLI command tree
JSON envelope
version command
```

No remote feature should be faked. Unimplemented commands should fail clearly.

---

## 44. Milestone 1 — host + doctor + exec

Implement:

```text
host resolution
OpenSSH process execution
ControlMaster namespace
doctor
exec
JSON/human output
```

Acceptance:

- a real client host → remote Linux/WSL2 call works;
- separate `rhost` processes reuse transport;
- stdout/stderr/exit code/timeout are correct.

Do not move on until this is solid.

---

## 45. Milestone 2 — sessions

Implement:

```text
tmux capability detection
create/list/close
managed metadata
output logging
session exec
session send/read
human attach
```

Acceptance:

- cwd/env persist across separate local CLI invocations;
- controller restart is irrelevant because no controller exists;
- session can be attached by a human;
- command boundary protocol is tested.

---

## 46. Milestone 3 — jobs

Implement:

```text
job start/list/status
remote metadata
stdout/stderr
offset logs
stop/kill
stale detection
```

Acceptance:

- a job survives CLI exit and SSH disconnect;
- remote process group can be stopped;
- completed jobs remain inspectable.

---

## 47. Milestone 4 — files

Implement:

```text
put/get
rsync-backed sync
dry-run
explicit delete
JSON result
```

Acceptance:

- source changes sync correctly;
- unrelated remote data is not deleted by default;
- destructive sync has a preview path.

---

## 48. Milestone 5 — status/watch

Implement:

```text
one-shot remote probe
Linux/WSL2 system model
optional NVIDIA probe
session/job aggregation
watch TUI
```

Acceptance:

- status works without NVIDIA;
- unsupported telemetry does not fail snapshot;
- watch survives temporary disconnect and reconstructs state.

---

## 49. Milestone 6 — hardening

Only after daily use:

```text
audit
output truncation policy
config migration
shell recovery
more robust process cancellation
installer
release signing/checksums
```

Then evaluate whether a local daemon is actually justified.

---

# Part XVII — deferred architecture

## 50. When a local daemon becomes justified

Do not add `rhostd` merely to make the architecture look service-oriented.

A daemon becomes justified if real usage needs one or more of:

- very high-frequency telemetry;
- continuous bidirectional PTY streaming;
- multiple local clients coordinating writes;
- event subscriptions rather than polling;
- richer human takeover semantics;
- persistent native SSH connections that outperform ControlMaster materially;
- local API consumers beyond the CLI.

Then the shape may become:

```text
Agent / Human
     │
   rhost CLI
     │ Unix socket
     ▼
   rhostd
     │
     ▼
Remote hosts
```

Even then, sessions/jobs should not become dependent on daemon survival unless intentionally redesigned.

---

## 51. When a remote runtime becomes justified

A remote daemon is a later step, not v0.1.

It becomes justified if tmux/nohup start limiting:

- precise PTY streaming;
- reliable process trees;
- resource accounting;
- event delivery;
- job isolation;
- Windows-native remote support;
- many concurrent clients.

A future `rhost-agentd` could expose a private protocol through an SSH tunnel.

SSH should remain the authentication/network boundary unless there is a compelling reason to replace it.

---

# Part XVIII — anti-goals

## 52. Things Codex should actively avoid

Do not:

- create `gpu-test`, `yolo-train`, or project-specific commands;
- make every remote action use a persistent shell;
- run long work as a blocking `exec`;
- store persistent state only in Go maps;
- implement an MCP server first;
- require a daemon for v0.1;
- duplicate `~/.ssh/config`;
- turn off host-key checking to make tests easier;
- auto-install tmux/rsync remotely;
- parse human-formatted output when structured data can exist;
- let `watch` become the owner of jobs/sessions;
- invent a broad plugin framework before a second backend exists;
- introduce interfaces only to make the code "clean";
- create a `utils` dumping ground;
- mix file-sync deletion into a default-safe command;
- hide unknown job/session states as success.

---

# Part XIX — Definition of done

## 53. First release

The first release is usable when a coding agent can execute this workflow through only the shipped Skill and binary:

```text
discover/doctor host
       ↓
sync code
       ↓
run stateless test
       ↓
inspect result
       ↓
open persistent interactive session when needed
       ↓
start durable long job
       ↓
continue other work
       ↓
poll logs/status after reconnect
```

And a human can independently run:

```text
rhost watch <host>
rhost session attach <host> <session>
```

without changing the runtime model.

The remote host must feel like a reusable compute node, not like a collection of handcrafted SSH commands.

---

# Part XX — Codex self-check

Before declaring an implementation task complete, ask:

- Did this change preserve the three distinct execution semantics?
- Does any promised persistent state die with the CLI process?
- Is OpenSSH still the source of truth for auth and host verification?
- Did I add a project-specific concept to a generic adapter?
- Does the agent have a JSON path for information I rendered for humans?
- Can reconnect rediscover the truth instead of relying on local memory?
- Can I deliberately break the feature and see the relevant test fail?
- Did I borrow an idea from Portal without accidentally borrowing its MCP/server lifecycle?
- Did I add abstraction before a second implementation required it?
- Did I test on the real client → remote Linux/WSL2 path for behavior that mocks cannot prove?

If a persistence feature has not survived a real process exit and reconnect, it is not implemented yet.
