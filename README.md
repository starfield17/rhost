# rhost

A general-purpose remote-host adapter for coding agents and humans.

`rhost` lets a coding agent on one machine treat an SSH-reachable machine as a
reusable execution node — without re-deriving SSH, quoting, timeout, and
persistence mechanics on every operation. It orchestrates your existing OpenSSH
configuration and never duplicates SSH authentication or host-key policy.

> Status: **v0.1.0-alpha.2**. The v0.1 surface is implemented and verified against
> a real remote Linux host over SSH: `hosts`, `doctor`, `exec`, `exec-many`,
> `session` (including `recover`), `job`, `fs` (transfer, and the remote
> read/write/patch/grep/glob helper), `tunnel`, `status`, `watch`, and a local
> `audit` trail. Still open: config migration, release signing, and the daemon
> evaluation. Releases carry build-provenance attestations — provenance, not a
> signature (see
> [`docs/architecture/16-milestones.md`](docs/architecture/16-milestones.md) §49).
> The verification runs the built binary as a
> **separate process per step**, so "persistent" means it survived an actual CLI
> process exit — including `SIGKILL` of the client, and an SSH connection closed
> underneath a running job — rather than an in-process simulation. Reproduce it
> with `make test-live-all` against your own target (see Development).
>
> Local side: macOS and Linux, which is what CI runs. Other OpenSSH platforms are
> untested. Remote side: an SSH-reachable Linux host — native, container, or WSL2.

## Install / build

```bash
curl -fsSL https://raw.githubusercontent.com/starfield17/rhost/main/scripts/install.sh | bash
```

The installer downloads the release binary for this platform, verifies it against
the release's own SHA-256 file, and only then replaces `~/.local/bin/rhost` — a
failed check leaves an existing install untouched. `RHOST_INSTALL_DIR` moves the
destination and `RHOST_VERSION` pins a version; without a pin it takes the newest
stable release, or the newest release of any kind while every release is a
prerelease.

Every release artifact is also attested: the attestation names the workflow, the
repository and the commit that produced the bytes, so a download can be checked
back to its source.

```bash
gh attestation verify rhost_<version>_<os>_<arch> --repo starfield17/rhost
```

From source:

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
rhost exec gpu --cwd '~/work/foo' -- pytest -q  # remote home, not local home
rhost exec gpu --json -- python check.py      # machine-readable result

rhost session create gpu --name debug --cwd '~/work/foo'
rhost session exec gpu debug -- 'cd src && pytest -q'   # state persists
rhost session send gpu debug --key C-c
rhost session read gpu debug --json --since 0
rhost session list gpu
rhost session attach gpu debug                # human, interactive
rhost session close gpu debug

rhost job start gpu --cwd '~/work/foo' -- python train.py  # survives disconnect
rhost job list gpu --json
rhost job status gpu <job-id> --json
rhost job logs gpu <job-id> --json --since 0  # byte cursor; pass data.next back
rhost job stop gpu <job-id>                   # SIGTERM to the process group
rhost job kill gpu <job-id>                   # SIGKILL to the process group

rhost fs put gpu ./model.py '~/work/foo/model.py'
rhost fs get gpu '~/work/foo/results.json' ./results.json
rhost fs sync gpu ./project '~/work/project' --dry-run   # the plan, nothing copied
rhost fs sync gpu ./project '~/work/project' --json      # apply; never deletes

rhost status gpu                              # one snapshot: system + sessions + jobs
rhost status gpu --json                       # ...as one JSON document
rhost watch gpu                               # live monitor, Ctrl-C to stop
rhost watch gpu --json --count 1              # one envelope, for a scripted probe

rhost audit --json                            # local audit log of remote operations

rhost version
```

Hosts are named exactly as you would name them to `ssh`: an alias from
`~/.ssh/config`, a `user@host`, or a bare hostname.

### Remote development tools

```bash
rhost fs read gpu '~/work/project/main.go' --json
rhost fs write gpu '~/work/project/new.txt' --from ./new.txt --json
rhost fs patch gpu '~/work/project/main.go' --patch ./patch.json --json
rhost fs grep gpu 'TODO' '~/work/project' --mode content --limit 100 --json
rhost fs glob gpu '*.go' '~/work/project' --json
rhost fs mirror gpu '~/work/project' ./download --dry-run --json
rhost fs put gpu ./artifact.bin '~/work/artifact.bin' --resume --json
rhost fs batch gpu --manifest ./transfers.json --json
rhost exec-many --host gpu --host build --parallel 2 --json -- 'uname -s'
rhost session recover gpu debug --json
rhost tunnel open gpu --kind local --listen localhost:8080 --destination localhost:8000 --json
rhost tunnel list --json
rhost tunnel close <id> --json
```

File read/write/patch require remote Python 3; grep/glob also require `rg`.
`doctor` reports these dependencies. No packages are installed automatically.
Reads default to 200 lines and 256 KiB, returning the complete file's SHA-256.
Search defaults to 100 records and 256 KiB, respects ignore rules, and supports
`--hidden`, `--no-ignore`, `--offset`, and `--limit`. Grep offers `--mode content`,
`files`, or `count`, plus `--context`, `--glob`, and `--ignore-case`. Results
include `truncated`; search pagination uses `next` as the next offset.

Writes create new files by default. To replace an existing file, pass its last
read hash as `--if-hash`. A new file is created private (`0600`) and an existing
one keeps its own permissions; `--mode 0755` names them explicitly, which is how a
written script becomes runnable. Patch JSON uses 1-based inclusive line ranges
against the original file:

```json
{"sha256":"<hash-from-read>","edits":[{"start":2,"end":3,"text":"replacement\n"}]}
```

Edits are UTF-8, limited to 8 MiB, and reject overlapping ranges and symlink
targets. A directory lock serializes rhost writers; same-directory temporary
files provide atomic replacement and preserve ordinary file permissions.
Hashes are checked before replacement and verified afterwards. An unrelated
editor does not participate in the lock, so this is not a filesystem-wide
compare-and-swap guarantee. On `FILE_CONFLICT`, read again before deciding how
to merge; never blindly reuse an old hash.

`mirror` downloads directory contents. `sync` and `mirror` only delete with
explicit `--delete`; preview with `--dry-run`. Destructive sync additionally
requires remote Python 3 to check the resolved destination. Existing symlinks
to broad targets are refused; concurrent external filesystem changes are not
locked throughout rsync. `--checksum` compares contents instead of timestamps.
Single-file `--resume` uses rsync partial files and verifies SHA-256 at completion;
`--checksum` also enables verified rsync transfer. These modes require rsync on
both sides and remote `sha256sum`. `resume_enabled` describes the requested mode,
not a claim that bytes were actually reused. `checksum_verified` reports success.

A batch manifest is an ordered array of
`{"op":"put","source":"./file","destination":"~/work/file"}` entries;
`get` reverses direction. Optional `resume` and `checksum` booleans apply per
entry. Results retain per-file failures; a partial failure exits 255.

`exec` and `exec-many` default to 1 MiB per output stream. Set
`--max-output-bytes 0` for unlimited output. Truncation flags and byte counts are
in JSON, and a large output does not discard the command's completion marker.
An execution deadline with unconfirmed cleanup may leave the remote command
running; inspect `timed_out` and `cleanup_confirmed` before retrying side effects.
Session timeouts attempt Ctrl-C and report `session_preserved`; `session recover`
interrupts and tests responsiveness without recreating the session.

`exec-many` preserves input ordering, defaults to four workers, and supports
`--serial`, `--delay` (per worker), and `--stop-on-error` (unstarted targets only).
Its aggregate exit code is 0 for all successful commands, 1 for remote failures
or skipped targets, and 255 for adapter failures. Each result retains its own
remote exit code. It is foreground orchestration, not a durable scheduler.

Tunnels use dedicated OpenSSH masters and survive CLI exit. They support local,
reverse and SOCKS forwarding, default to loopback binding, and require explicit
`--allow-exposure` for other binds. `list` checks the master and reports stale
records; it does not prove the forwarded service is healthy. Closing a tunnel
does not close the shared exec transport. Tunnels do not restart after a reboot.

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
- [`skills/rhost/`](skills/rhost/SKILL.md) — the agent-facing skill: hard rules
  in `SKILL.md`, and the command cookbook, recovery recipes and safety notes in
  `references/`. The directory is self-contained: copy or symlink it into an
  agent's skills directory as-is.
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
make check   # gofmt + vet + unit tests + portability scan + release contract

# The release artifacts, exactly as CI builds them (pass DATE=<timestamp> to
# reproduce a published build byte for byte):
make dist VERSION=0.1.0

# Live tests are opt-in: name your own target, nothing is hardcoded.
RHOST_TEST_HOST=<user>@<host> make test-live           # exec, timeout, doctor, transport reuse
RHOST_TEST_HOST=<user>@<host> make test-live-session   # persistence across CLI processes, busy-pane refusal
RHOST_TEST_HOST=<user>@<host> make test-live-jobs      # persistence, signals, process identity, log cursors
RHOST_TEST_HOST=<user>@<host> make test-live-fs        # put/get round-trip, rsync plan, --delete, cache root with a space
RHOST_TEST_HOST=<user>@<host> make test-live-status    # status snapshot, watch stream, offline
RHOST_TEST_HOST=<user>@<host> make test-live-tools     # fs tools, verified transfer, exec-many, tunnels
RHOST_TEST_HOST=<user>@<host> make test-live-all       # every live suite
```

Live tests build `rhost` and invoke it as a **separate process per step**: state
that is promised to outlive the CLI is only proven by exiting and restarting it.
The embedded remote helper (`internal/fileops/remote.py`) has its own suite
(`remote_test.py`), run from `make test` through a Go wrapper so it cannot drift
quietly. Python 3 is a mandatory local test dependency; its absence fails the
gate instead of silently reducing coverage.
The target host is only ever an environment variable at invocation — never a
default, and never written into a file (`AGENTS.md` §1).

`rhost` can do exactly what the current OS user can do through the configured
SSH identity, and no more — see
[`skills/rhost/references/SAFETY.md`](skills/rhost/references/SAFETY.md).

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
per-user root automatically; a cache path containing whitespace moves there too,
so the ControlPath never has to be quoted inside rsync's `-e` string.

## A note on remote login shells

`sshd` runs a client command through the remote account's **login shell**.
If
that shell has a slow startup file (for example a `fish` config that activates
conda on every start), every `exec` pays that cost. rhost reports the login
shell in `doctor` and deliberately does not bypass it — OpenSSH and the user's
shell configuration remain the source of truth.
