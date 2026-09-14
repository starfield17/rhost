# rhost

**Remote execution for coding agents, built on OpenSSH.**

*The agent stays local. The work doesn't have to.*

A coding agent already knows how to use a shell. rhost carries that command
model to any SSH-reachable Linux machine without installing an agent, running a
daemon, or introducing another authentication system.

```bash
rhost exec gpu --cwd '~/work/project' --command 'cargo test --locked'
```

Foreground commands behave like commands: stdin goes in, stdout and stderr come
back, and rhost exits with the remote command's status. OpenSSH still resolves
the target, authenticates it, checks its host key, follows ProxyJump, and reuses
connections.

```text
local coding agent
  |-- rhost edge --> remote Linux host
  `-- rhost gpu  --> GPU workstation
```

Direct execution needs remote `bash`, `setsid`, `ps`, and a base64 decoder.
Optional operations need more: sessions use `tmux` and `flock`, text helpers use
Python 3, and resumable or checksum-aware transfers use rsync. Run `rhost doctor
<host> --json` when you need to see what a host supports.

## Install

The release installer supports macOS and Linux on amd64 and arm64. It verifies
the downloaded binary against the published SHA-256 file before atomically
replacing an existing installation.

```bash
curl -fsSL https://raw.githubusercontent.com/starfield17/rhost/main/scripts/install.sh | bash
```

By default the binary goes to `~/.local/bin`. Set `RHOST_INSTALL_DIR` to choose
another directory, or `RHOST_VERSION` to pin a published release.

To build from source with Rust 1.85 or newer:

```bash
git clone https://github.com/starfield17/rhost.git
cd rhost
make build
install -m 755 target/debug/rhost ~/.local/bin/rhost
```

## Start with an ordinary command

Use any target that already works with `ssh`: an OpenSSH config alias,
`user@example-host`, or a bare hostname.

```bash
# Inspect an edge host.
rhost exec edge --command 'uname -a && systemctl --failed'

# Check a GPU worker.
rhost exec gpu --command 'nvidia-smi'

# Run the same command an agent would run locally.
rhost exec gpu --cwd '~/work/project' --command 'pytest -q'

# Forward stdin and preserve normal pipeline behavior.
printf 'hello\n' | rhost exec edge --command 'cat'
rhost exec gpu --command 'cat results.txt' | sort
```

The `--command` value is one complete remote shell program. Pipelines,
redirects, variables, and compound syntax inside it run in a fresh remote login
bash. Shell syntax outside it remains local. `-c` is the short form; for a
larger local script use `--command-file ./script.sh`, which leaves stdin free for
the remote program.

Human mode streams output and has no default deadline or output cap. Add
`--timeout 30s` when the command has a meaningful bound. Use `--json` when the
caller needs one versioned result rather than terminal output. With JSON,
`--stream` mirrors live command output to stderr while stdout remains exactly
one final envelope. Add `--fresh` when diagnosis needs an independent SSH
connection.

## Why not just SSH?

For a human, `ssh gpu ...` is often enough. For an agent, the edges decide
whether it can classify a result and retry safely:

- stdin, stdout, stderr, and the remote exit status keep ordinary process
  behavior;
- a remote command failure is distinct from usage, transport, and local output
  failures;
- a timeout says whether cleanup was confirmed instead of pretending an
  uncertain mutation is safe to repeat;
- sessions and tunnels survive the rhost invocation that created them;
- OpenSSH remains authoritative for configuration, authentication, host keys,
  ProxyJump, and connection reuse;
- replacing remote text can require the hash returned by the preceding read.

rhost stays below the agent protocol layer. It is not an SSH client or a
resident control plane. It is a remote process boundary that a coding agent can
invoke like any other command-line program.

## When foreground execution is not enough

Direct execution is the default. Reach for a specialized operation only when
the work needs an additional guarantee.

| Requirement | Operation |
|---|---|
| A shell, REPL, or debugger must keep state | `rhost session` |
| Bytes must cross local and remote filesystems | `rhost fs` |
| A text edit needs an atomic hash precondition | `rhost fs read/write/patch` |
| A port forward must survive its creator | `rhost tunnel` |
| SSH or remote dependency diagnosis | `rhost doctor` |

For work that only needs to outlive one blocking tool call, let the coding agent
run an ordinary `rhost exec` process in its own background facility. The rhost
process remains the foreground owner, so output and exit status keep their
normal meaning.

Work that must survive the coding agent itself belongs to a scheduler already
installed on the remote host. Invoke that scheduler explicitly, for example:

```bash
rhost exec gpu --cwd '~/work/project' \
  --command 'systemd-run --user --unit=training --collect python train.py --config configs/train.yaml'
```

rhost does not select or install a scheduler. Use the host's own systemd, Slurm,
Kubernetes, tmux, or other established runtime when its lifecycle guarantees
are required. A persistent rhost session is for terminal state, not a generic
scheduler.

File operations and sessions remain explicit:

```bash
rhost fs read gpu '~/work/project/src/main.rs' --json
rhost fs write gpu '~/work/project/src/main.rs' --from ./main.rs --if-hash <sha256>

rhost session create gpu --name debug --cwd '~/work/project' --json
rhost session exec gpu debug --command 'python3 -m pdb app.py' --json
```

Creating a file needs no hash. Replacing or patching one requires the SHA-256
from the last `fs read`. Sync and mirror delete only with explicit `--delete`.
Remote search stays an ordinary command:

```bash
rhost exec gpu --cwd '~/work/project' --command 'rg TODO src'
```

`session attach` is deliberately refused with `USAGE_ERROR`: schema v2 has no
honest way to represent terminal attachment in a single result envelope.

## What rhost does not own

rhost does not manage SSH keys, keep a host database, install a remote agent,
run a daemon, or define a deployment language.

OpenSSH owns connections and authentication. Remote tmux owns persistent
terminals. Remote schedulers own scheduled work. The filesystem owns files.
rhost owns the contract between those components and the caller.

## Structured results

Every agent-visible behavior has a `--json` path. JSON mode writes exactly one
schema-v2 document to stdout, never mixes progress or command output into it,
and provides stable `error.code` values.

For execution results, inspect:

- `data.execution.status`: `completed`, `unknown`, or `not_started`;
- `data.execution.exit_code`: present only when remote completion is known;
- `data.output.stdout.content` and `data.output.stderr.content`, together with
  their byte counts and truncation flags;
- `data.cleanup.status`: `not_attempted`, `confirmed_stopped`, or `unconfirmed`.

| Process status | Meaning |
|---|---|
| 0-254 | Remote command status when completion is known |
| 124 | rhost execution deadline |
| 130 / 143 | rhost handled SIGINT / SIGTERM |
| 255 | Adapter, transport, usage, or local delivery failure |

A remote program can return the same numeric values. In ambiguous cases inspect
`ok`, `data.execution`, and `error.code` rather than branching on English text.
Missing completion evidence is reported conservatively as
`REMOTE_EXECUTION_UNKNOWN`; confirmed process-group cleanup does not by itself
make a side effect safe to replay.

## Use as an agent skill

The repository includes a concise skill that teaches an agent when an ordinary
command is enough and when sessions, file operations, or tunnels provide a
needed guarantee:

```bash
npx skills add starfield17/rhost --skill rhost
```

The canonical skill is also listed on
[skills.sh](https://skills.sh/starfield17/rhost/rhost). Codex users can ask the
built-in skill installer to install this GitHub directory; the new skill is
available on the next turn:

```text
$skill-installer install the rhost skill from
https://github.com/starfield17/rhost/tree/main/skills/rhost
```

The root `plugin.json` is the portable Agent Plugin entry point, while
`.codex-plugin/plugin.json` exposes the same skill to Codex plugin discovery.
Both package the skill and require the `rhost` CLI to be installed separately.

## Design and verification

The CLI process owns no durable remote work. Killing and restarting it does not
erase state it promised to preserve.

- [Project overview](docs/PROJECT_OVERVIEW.md)
- [Architecture and ownership](docs/ARCHITECTURE.md)
- [Agent skill](skills/rhost/SKILL.md)
- [JSON envelope schema](schemas/result-v2.schema.json)
- [Contributor rules](AGENTS.md)

```bash
make test-smoke
make check

RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-session
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-all
```

`test-smoke` is the complete local Rust black-box suite and cannot contact a
real host. `test-live-all` is the serial, Rust-native real-SSH release gate. The
frozen Go harness remains available only through `make test-conformance` and the
explicitly named `test-legacy-live-*` targets; it is historical evidence, not a
Rust release dependency.

Live suites always take their target and binary from `RHOST_TEST_HOST` and
`RHOST_BIN`; the repository has no machine-specific default. The v1 schema is
retained as wire history. Current command availability is defined by `rhost
--help`, and the implemented and remotely verified status is tracked in
[`docs/RUST_MIGRATION.md`](docs/RUST_MIGRATION.md).
