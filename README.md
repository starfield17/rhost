# rhost

A general-purpose remote-host adapter for coding agents and humans.

`rhost` lets a coding agent on one machine treat an SSH-reachable machine as a
reusable execution node — without re-deriving SSH, quoting, timeout, and
persistence mechanics on every operation. It orchestrates your existing OpenSSH
configuration and never duplicates SSH authentication or host-key policy.

> Status: early. Milestone 0 (skeleton), Milestone 1 (host resolution,
> `doctor`, `exec`), Milestone 2 (persistent tmux-backed `session`),
> Milestone 3 (detached `job`), Milestone 4 (file transfer `fs`) and Milestone 5
> (system `status` and live `watch`) are implemented and verified against a real
> remote Linux host over SSH. The verification runs the built binary as a
> **separate process per step**, so "persistent" means it survived an actual CLI
> process exit — including `SIGKILL` of the client, and an SSH connection closed
> underneath a running job — rather than an in-process simulation. Reproduce it
> with `make test-live-all` against your own target (see Development).

## Install / build

```bash
go build -o bin/rhost ./cmd/rhost
# or
make build
```

## Usage

```bash
rhost hosts                                   # list SSH config aliases
rhost doctor gpu                              # probe a host's capabilities
rhost exec gpu -- pwd                         # stateless foreground command
rhost exec gpu --cwd ~/work/foo -- pytest -q  # with an explicit cwd
rhost exec gpu --json -- python check.py      # machine-readable result

rhost session create gpu --name debug --cwd ~/work/foo
rhost session exec gpu debug -- 'cd src && pytest -q'   # state persists
rhost session send gpu debug --key C-c
rhost session read gpu debug --json --since 0
rhost session list gpu
rhost session attach gpu debug                # human, interactive
rhost session close gpu debug

rhost job start gpu --cwd ~/work/foo -- python train.py  # survives disconnect
rhost job list gpu --json
rhost job status gpu <job-id> --json
rhost job logs gpu <job-id> --json --since 0  # byte cursor; pass data.next back
rhost job stop gpu <job-id>                   # SIGTERM to the process group
rhost job kill gpu <job-id>                   # SIGKILL to the process group

rhost fs put gpu ./model.py ~/work/foo/model.py
rhost fs get gpu ~/work/foo/results.json ./results.json
rhost fs sync gpu ./project ~/work/project --dry-run   # the plan, nothing copied
rhost fs sync gpu ./project ~/work/project --json      # apply; never deletes

rhost status gpu                              # one snapshot: system + sessions + jobs
rhost status gpu --json                       # ...as one JSON document
rhost watch gpu                               # live monitor, Ctrl-C to stop
rhost watch gpu --json --count 1              # one envelope, for a scripted probe

rhost audit --json                            # local audit log of remote operations

rhost version
```

Hosts are named exactly as you would name them to `ssh`: an alias from
`~/.ssh/config`, a `user@host`, or a bare hostname.

### Exit codes

| status | meaning |
|---|---|
| 0–254 | the remote command's own exit status |
| 124 | foreground timeout (`REMOTE_COMMAND_TIMEOUT`) |
| 255 | adapter failure — transport, validation, or usage (see `error.code` in `--json`) |

124 is the one overlap with remote statuses, so an ambiguous case is always
resolved from `--json`: `ok`, `data.exit_code`, `error.code`. A remote command
that exits 255 is indistinguishable by status alone for the same reason.

## Design

- [`docs/PROJECT_OVERVIEW.md`](docs/PROJECT_OVERVIEW.md) — what the project is
  and why.
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — the map: each Part links to
  one small file under [`docs/architecture/`](docs/architecture/), holding the
  implementation framework, milestones, and acceptance criteria.
- [`AGENTS.md`](AGENTS.md) — contributor rules (portability, persistence,
  verification).
- [`SKILL.md`](SKILL.md) — the agent-facing usage policy.
- [`schemas/result-v1.schema.json`](schemas/result-v1.schema.json) — the JSON
  envelope contract.

The one invariant behind the architecture: **anything promised to survive a
CLI invocation must be owned outside the CLI process.** Connection reuse is
owned by OpenSSH ControlMaster; sessions by remote tmux; jobs by a detached
remote process whose state is a directory of remote files. rhost itself owns
nothing durable — which is why `fs` is foreground: a copy that was promised to
outlive the CLI would need a remote owner, and that is a later milestone.

## Development

```bash
make check   # gofmt + vet + unit tests + portability scan

# Live tests are opt-in: name your own target, nothing is hardcoded.
RHOST_TEST_HOST=<user>@<host> make test-live           # exec, timeout, doctor, transport reuse
RHOST_TEST_HOST=<user>@<host> make test-live-session   # session persistence across CLI processes
RHOST_TEST_HOST=<user>@<host> make test-live-jobs      # job persistence, signals, log cursors
RHOST_TEST_HOST=<user>@<host> make test-live-fs        # put/get round-trip, rsync plan, --delete
RHOST_TEST_HOST=<user>@<host> make test-live-status    # status snapshot, watch stream, offline
RHOST_TEST_HOST=<user>@<host> make test-live-all       # every live suite
```

Live tests build `rhost` and invoke it as a **separate process per step**: state
that is promised to outlive the CLI is only proven by exiting and restarting it.
The target host is only ever an environment variable at invocation — never a
default, and never written into a file (`AGENTS.md` §1).

`rhost` can do exactly what the current OS user can do through the configured
SSH identity, and no more — see [`SKILL.md`](SKILL.md) §5.

## Troubleshooting

If a command fails with `SSH_UNREACHABLE` and the message says there was "no
diagnostic", OpenSSH suppressed its own reason at the default log level. Re-run
once with more verbosity to see it:

```bash
RHOST_SSH_LOG_LEVEL=VERBOSE rhost doctor <host>
```

This affects diagnosis only: rhost never changes host-key policy or authentication.

Transport sockets live under the local cache dir. If that path is very deep,
OpenSSH's Unix-socket length limit applies and rhost falls back to a short
per-user root automatically.

## A note on remote login shells

`sshd` runs a client command through the remote account's **login shell**.
If
that shell has a slow startup file (for example a `fish` config that activates
conda on every start), every `exec` pays that cost. rhost reports the login
shell in `doctor` and deliberately does not bypass it — OpenSSH and the user's
shell configuration remain the source of truth.
