# rhost

A general-purpose remote-host adapter for coding agents and humans.

`rhost` lets a coding agent on one machine treat an SSH-reachable machine as a
reusable execution node — without re-deriving SSH, quoting, timeout, and
persistence mechanics on every operation. It orchestrates your existing OpenSSH
configuration and never duplicates SSH authentication or host-key policy.

> Status: early. Milestone 0 (skeleton) and Milestone 1 (host resolution,
> `doctor`, `exec`) are implemented and verified against a real Mac → Linux
> (Orange Pi 5) path. Sessions, jobs, file sync, and status/watch are designed
> but not yet implemented.

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
rhost version
```

Hosts are named exactly as you would name them to `ssh`: an alias from
`~/.ssh/config`, a `user@host`, or a bare hostname.

### Exit codes

| status | meaning |
|---|---|
| 0–254 | the remote command's own exit status |
| 255 | adapter/transport failure (see `error.code` in `--json`) |
| 124 | foreground timeout |

## Design

- [`docs/PROJECT_OVERVIEW.md`](docs/PROJECT_OVERVIEW.md) — what the project is
  and why.
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — implementation framework,
  milestones, and acceptance criteria.
- [`SKILL.md`](SKILL.md) — the agent-facing usage policy.
- [`schemas/result-v1.schema.json`](schemas/result-v1.schema.json) — the JSON
  envelope contract.

The one invariant behind the architecture: **anything promised to survive a
CLI invocation must be owned outside the CLI process.** Connection reuse is
owned by OpenSSH ControlMaster; later, sessions by remote tmux and jobs by
remote processes with durable remote metadata.

## Development

```bash
make test      # unit tests
make vet
RHOST_TEST_LIVE=1 RHOST_TEST_HOST=user@host make test-live
```

Live tests are opt-in and never run by default.

## A note on remote login shells

`sshd` runs a client command through the remote account's **login shell**. If
that shell has a slow startup file (for example a `fish` config that activates
conda on every start), every `exec` pays that cost. rhost reports the login
shell in `doctor` and deliberately does not bypass it — OpenSSH and the user's
shell configuration remain the source of truth.
