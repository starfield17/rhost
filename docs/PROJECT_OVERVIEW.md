# Remote Host Adapter — Project Overview

> Working name: **rhost**  
> Status: implemented through Milestone 4 (`hosts`, `doctor`, `exec`, `session`,  
> `job`, `fs`); Milestone 5 (status/watch) is designed but not built  
> Primary form: **Skill + source repository + single release binary**  
> Local side: any machine with a standard OpenSSH client  
> Remote side: any SSH-reachable Linux host (native, container, or WSL2)

---

## 1. What this project is

`rhost` is a **general-purpose remote host adapter for coding agents and humans**.

Its job is to let a coding agent running on one computer treat another SSH-reachable computer as a reusable execution node, without forcing the agent to reason about low-level SSH mechanics on every operation.

The initial motivating setup is:

```text
local machine
  ├─ coding agent
  ├─ source editing / reasoning
  └─ rhost
        │
        │ SSH
        ▼
remote Linux host
  ├─ stronger CPU
  ├─ GPU / CUDA when present
  ├─ test environments
  ├─ long-running jobs
  └─ interactive shells
```

These are roles, not products. The client side is any machine that runs an
OpenSSH client; the remote side is any SSH-reachable Linux host. Specific
hardware, board models, addresses, and accounts never belong in this repository
(`AGENTS.md` §1).

The project is **not** specific to YOLO, CUDA, machine learning, or WSL2. Those are early use cases. The same adapter should later work for compilation, benchmarks, data processing, services, REPLs, debuggers, test suites, ARM boards, Linux servers, cloud VMs, and other remote hosts.

The fundamental abstraction is:

> A remote host is a compute resource with execution, files, interactive sessions, durable jobs, and observable system state.

---

## 2. The failure this project prevents

Without an adapter, a coding agent tends to accumulate ad-hoc remote logic:

```text
ssh host "..."
scp ...
rsync ...
ssh host "tmux ..."
ssh host "nvidia-smi ..."
ssh host "ps ..."
```

Each project then invents its own conventions for:

- connection reuse;
- quoting;
- working directories;
- environment variables;
- long-running jobs;
- session recovery;
- log polling;
- file synchronization;
- GPU/system monitoring;
- errors and timeouts;
- machine-readable output.

The result is not that SSH is incapable. The problem is that **transport details leak into agent reasoning**.

`rhost` should make the correct remote-operation semantics explicit and reusable.

---

## 3. The one rule

> **If state must survive one `rhost` CLI invocation, that state may not live only inside the CLI process.**

This rule determines the architecture.

A CLI process is disposable. It may start, perform one operation, and exit.

Therefore:

| State | Owner |
|---|---|
| SSH transport reuse | OpenSSH ControlMaster process / socket |
| Interactive terminal state | remote tmux session |
| Long-running computation | remote detached process + remote metadata |
| Host configuration | local config / OpenSSH config |
| Durable adapter metadata | filesystem, not process memory |
| Human monitor state | reconstructed from remote/local durable state |

This is deliberately different from a long-running MCP server that owns live SSH objects in memory.

---

## 4. What the user should experience

For a human:

```bash
rhost status gpu
rhost watch gpu
rhost exec gpu --cwd ~/work/foo -- pytest -q
rhost session create gpu --name debug
rhost session attach gpu debug
rhost job start gpu --cwd ~/work/foo -- python train.py
rhost job logs gpu <job-id>
rhost fs sync gpu ./foo ~/work/foo
```

For an agent, the same binary should expose stable structured output:

```bash
rhost status gpu --json
rhost exec gpu --json --cwd ~/work/foo -- pytest -q
rhost job status gpu <job-id> --json
```

The agent and the human use the **same control plane**. The difference is presentation:

```text
                 rhost core
                /          \
               /            \
      structured JSON      human CLI/TUI
          for agent         for operator
```

There should be no separate YOLO-specific, GPU-test-specific, or project-specific command layer in the core product.

---

## 5. The three execution semantics

The project must keep these concepts separate.

### 5.1 `exec` — stateless foreground execution

Use for the default case.

```text
rhost exec
    │
    ▼
fresh remote command channel
    │
    ▼
stdout + stderr + exit code
```

Properties:

- explicit `cwd`, environment, timeout;
- no dependency on previous shell history;
- suitable for tests, builds, probes, git commands, short scripts;
- easy to retry, log, audit, and reason about;
- should reuse the SSH transport without reusing shell state.

This should be the agent's default mode.

### 5.2 `session` — persistent interactive state

Use only when state across interactions is itself useful:

- REPL;
- debugger;
- interactive CLI;
- exploratory shell work;
- human/agent shared terminal.

Properties:

- remote tmux-backed lifetime;
- persistent cwd/env/shell process;
- survives local CLI exit and SSH disconnect;
- can be listed and reattached;
- supports both structured command execution and raw terminal input;
- can be attached directly by a human.

A session is not the default execution mechanism.

### 5.3 `job` — durable background work

Use when a process must survive the foreground control connection:

- model training;
- large builds;
- benchmarks;
- data processing;
- long test suites;
- development servers.

Properties:

- returns a stable job ID quickly;
- process continues after `rhost` exits;
- stdout/stderr are persisted remotely;
- status/logs can be polled later;
- can be signaled or stopped;
- does not require a persistent interactive shell.

A long job is not a long `exec`, and it is not merely a tmux window.

---

## 6. Generic monitoring is a first-class feature

The project should include a generic host monitor, but monitoring must remain **host-oriented**, not project-oriented.

Human-facing:

```bash
rhost watch gpu
```

Conceptual view:

```text
REMOTE HOST: gpu
────────────────────────────────────────
Connection    online · RTT 2.8 ms
OS            Linux (or WSL2)
Uptime        3d 11h
CPU           18%
Memory        11.2 / 32 GiB
Disk          380 / 950 GiB

Accelerators
NVIDIA RTX 4060
GPU           93%
VRAM          6.2 / 8.0 GiB

Managed sessions
dev            alive     ~/work/foo
debug          alive     ~/work/bar

Managed jobs
j_01...        running   2h17m
j_00...        exited    code=0
────────────────────────────────────────
```

Agent-facing:

```json
{
  "host": "gpu",
  "online": true,
  "system": {},
  "accelerators": [],
  "sessions": [],
  "jobs": []
}
```

The monitor should know about processes and resources, but not about YOLO metrics, PyTorch epochs, application business state, or any particular project.

---

## 7. Project form

The intended distribution model is:

```text
repository
├── SKILL.md
├── README.md
├── docs/
│   ├── PROJECT_OVERVIEW.md
│   └── ARCHITECTURE.md
├── cmd/
│   └── rhost/
├── internal/
│   └── ...
├── schemas/
│   └── ...
├── scripts/
│   └── install.sh
└── .github/
    └── workflows/
        └── release.yml
```

For Go, prefer normal Go repository conventions (`cmd/`, `internal/`, packages at module root) instead of creating a literal `src/` directory only for symmetry.

GitHub Releases should publish platform binaries such as:

```text
rhost_<version>_darwin_arm64
rhost_<version>_darwin_amd64
rhost_<version>_linux_amd64
rhost_<version>_linux_arm64
```

The first priority target is `darwin/arm64`.

"Single binary" means a single `rhost` artifact to install. It does **not** require v0.1 to be hermetic: the first implementation may intentionally use the local OpenSSH client and remote standard utilities such as `tmux`, `bash`, `nohup`, and `setsid`.

---

## 8. Why Skill + CLI instead of MCP-first

MCP can be a useful frontend, but it should not define the architecture.

The core workflow is already expressible as:

```text
coding agent
    │
    │ reads SKILL.md
    ▼
rhost CLI
    │
    ▼
SSH / remote runtime primitives
```

Advantages:

- usable by any coding agent that can execute shell commands;
- usable directly by humans;
- no mandatory JSON-RPC server lifecycle;
- no MCP server setup per client;
- no dependency on one agent ecosystem;
- easy to debug from an ordinary terminal;
- release and versioning are straightforward;
- core behavior can later be wrapped by MCP without redesigning it.

A future optional frontend may exist:

```text
                rhost core
               /          \
          rhost CLI      rhost-mcp
```

But `rhost-mcp` would be an adapter around the core, not the core itself.

---

## 9. The role of `SKILL.md`

`SKILL.md` is the **agent policy and usage layer**, not the implementation.

It should tell an agent:

- when remote execution is appropriate;
- how to discover configured hosts;
- to prefer `exec` for ordinary commands;
- to use `session` only for stateful/interactive work;
- to use `job` for work that must survive disconnects or exceeds foreground time limits;
- to prefer `--json` output;
- how to sync files safely;
- how to inspect a host before expensive work;
- how to recover after a disconnect;
- what command to run when the environment is unhealthy (`rhost doctor`);
- that `rhost` grants no authority beyond the local user's SSH credentials.

The Skill should avoid embedding SSH implementation details that the binary already knows.

---

## 10. Authentication and authority model

`rhost` must not invent a second credential system in v0.1.

OpenSSH remains responsible for:

- private keys;
- ssh-agent;
- `known_hosts`;
- host key verification;
- ProxyJump / bastions;
- user names and ports;
- key selection;
- existing SSH configuration.

The adapter should reuse the operator's existing `~/.ssh/config`.

Conceptually:

```text
rhost host alias
      │
      ▼
OpenSSH configuration
      │
      ▼
actual network/authentication path
```

`rhost` may add adapter-specific metadata for a host, but should not duplicate private keys or passwords.

The authority boundary is simple:

> `rhost` can do what the current OS user can do through the configured SSH identity, and no more.

---

## 11. Initial scope

### v0.1 should include

- host discovery/configuration;
- connectivity diagnostics;
- foreground `exec`;
- SSH connection reuse;
- tmux-backed persistent sessions;
- durable detached jobs;
- put/get/sync file operations;
- generic system status;
- NVIDIA telemetry when available;
- machine-readable JSON;
- human-readable CLI;
- generic live `watch`;
- local binaries for the primary client platforms (`darwin/arm64` first);
- Linux remote support, including WSL2;
- deterministic tests for command/result parsing and persistence discovery.

### Explicitly not required for v0.1

- MCP server;
- remote daemon;
- remote desktop / GUI automation;
- project-specific commands;
- job scheduler with priorities/queues;
- Kubernetes/Slurm orchestration;
- automatic package installation;
- password vault;
- sudo password handling;
- Windows-native remote shell backend;
- multi-user access control;
- cloud provisioning;
- full browser UI.

These can be considered only after real usage proves a need.

---

## 12. Reference implementation: `portal-mcp-server`

Codex will have a local checkout of `portal-mcp-server`.

Treat it as a **reference implementation and idea source**, not as the target architecture.

Useful ideas to study include:

- SSH connection reuse / connection management;
- the separation between stateless exec, persistent shell, and background job semantics;
- PTY command-boundary detection;
- timeout, cancellation, and recovery behavior;
- background job metadata and incremental log polling;
- file transfer abstractions;
- security/audit separation from execution logic;
- structured tool results.

Do **not** copy the MCP-first lifecycle assumption.

In particular, a Portal-style persistent shell may be owned by a long-running server process. `rhost` v0.1 cannot depend on that because each CLI invocation is disposable. Persistent sessions must therefore be owned remotely (tmux or a later remote runtime), not by an in-memory local object.

When the local Portal source and this document disagree about product shape, this document wins.

---

## 13. Success scenario

The project is successful when the following feels ordinary.

A coding agent on the local machine changes code locally, then:

```bash
rhost fs sync gpu ./project ~/work/project
rhost exec gpu --json --cwd ~/work/project -- pytest -q
```

It sees a real exit code and diagnostics, changes the code, and repeats.

For a long task:

```bash
rhost job start gpu --json --cwd ~/work/project -- python train.py
```

The CLI returns a job ID and exits. The agent can continue other work.

Later:

```bash
rhost job status gpu <job-id> --json
rhost job logs gpu <job-id> --since <offset> --json
rhost status gpu --json
```

The local machine may have lost and regained network in between. The remote job
is still discoverable.

Meanwhile the human can run:

```bash
rhost watch gpu
```

and, when needed:

```bash
rhost session attach gpu debug
```

The human and the agent do not need separate remote-control systems.

---

## 14. Definition of done for the first usable release

A release is not "done" because commands compile.

It is done when all of the following are demonstrated against a real SSH-accessible Linux host:

1. Ten consecutive `exec` calls reuse the SSH transport and return correct stdout, stderr, exit code, and timeout behavior.
2. A tmux-backed session preserves cwd and environment across separate `rhost` process invocations.
3. The local CLI can be killed, restarted, and still rediscover that session.
4. A job continues after the starting `rhost` process exits and after the SSH connection closes.
5. The job can later be rediscovered, inspected, tailed, and stopped.
6. File sync changes only intended paths and has an explicit dry-run or preview path for destructive sync behavior.
7. `status --json` is parseable and stable.
8. `watch` can observe connection/system/session/job state without owning any of that state.
9. Host key verification is not silently disabled.
10. No private key or password is persisted by `rhost`.

---

## 15. Self-check for implementation decisions

Before adding a feature, ask:

- Is this capability generic to a remote host, or is it really project-specific?
- Does it belong to `exec`, `session`, `job`, `fs`, or `status`, or are we inventing a fourth execution model?
- If the CLI dies immediately after returning, does the promised state still exist?
- Are we putting information in `SKILL.md` that the binary could expose mechanically?
- Are we duplicating SSH authentication/configuration instead of using OpenSSH?
- Are human-friendly terminal output and agent-friendly structured output being kept separate?
- Are we introducing a daemon because it is necessary, or because a long-running process feels architecturally neat?
- Could a future MCP frontend call the same core without changing semantics?
- Can the behavior be proven against a real remote host, not only unit-tested locally?

If the answer to the last question is no, the feature is not yet part of the reliable adapter.
