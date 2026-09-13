# rhost

**Remote execution for coding agents, built on OpenSSH.**

*The agent stays local. The work doesn't have to.*

A coding agent already knows how to use a shell. rhost carries that command
model to any SSH-reachable Linux machine without installing an agent, running a
daemon, or introducing another authentication system.

```bash
rhost exec gpu --cwd '~/work/project' --command 'pytest -q'
```

Direct execution requires remote `bash`, `setsid`, and a base64 decoder.
Optional features need more: sessions use `tmux`/`flock`, text helpers use
Python 3, and sync uses rsync. Use `rhost doctor <host> --json` for diagnosis.

Foreground commands behave like commands: stdin goes in, stdout and stderr come
back, and rhost exits with the remote command's status.

```text
local coding agent
  |-- rhost edge --> edge Linux host
  `-- rhost gpu  --> GPU workstation
```

## Install

The release installer supports macOS and Linux on amd64 and arm64. It verifies
the downloaded binary against the published SHA-256 file before replacing an
existing installation.

```bash
curl -fsSL https://raw.githubusercontent.com/starfield17/rhost/main/scripts/install.sh | bash
```

By default the binary is installed to `~/.local/bin`. Set `RHOST_INSTALL_DIR`
to choose another directory, or `RHOST_VERSION` to pin a release.

To build from source:

```bash
git clone https://github.com/starfield17/rhost.git
cd rhost
make build
install -m 755 bin/rhost ~/.local/bin/rhost
```

## Start with an ordinary command

Use a target that already works with `ssh`: an OpenSSH config alias,
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
redirects, variables, and compound syntax inside that value run in a fresh
remote login-bash context. Shell syntax outside the value remains local. Local
flags may appear before or after `--command`; use `-c` as its short form.

Human mode streams output and has no default deadline or output cap. Add
`--timeout 30s` when a command has a meaningful bound. Use `--json` when the
caller needs a versioned result envelope rather than terminal output.

## Why not just SSH?

For a human, `ssh gpu ...` is often enough. For an agent, the edges determine
whether it can retry safely and classify a result correctly:

- stdin, stdout, stderr, and the remote exit status retain ordinary process
  behavior;
- remote command failure is distinguishable from usage and transport failure;
- a timeout reports whether remote cleanup was confirmed instead of pretending
  an uncertain mutation failed;
- durable jobs and sessions survive the rhost process and SSH connection;
- OpenSSH configuration, authentication, host keys, ProxyJump, and connection
  reuse remain authoritative;
- replacing remote text can require the hash returned by the preceding read.

rhost stays below the agent protocol layer. It is not an SSH client or a
resident MCP control plane; it is a remote process boundary that coding agents
can invoke like any other command-line program.

## When foreground execution is not enough

The direct form is the default. Reach for a specialized operation only when the
work needs an additional guarantee.

| Requirement | Operation |
|---|---|
| Work must outlive this CLI or SSH connection | `rhost job` |
| A shell, REPL, or debugger must keep state | `rhost session` |
| Bytes must cross local and remote filesystems | `rhost fs` |
| A remote text edit needs an atomic hash precondition | `rhost fs read/write/patch` |
| A port forward must survive its creating invocation | `rhost tunnel` |
| SSH or remote dependency diagnosis | `rhost doctor` |

For example, start a training command that must keep running after the agent's
invocation ends, then inspect it later:

```bash
rhost job start gpu --name training --cwd '~/work/project' \
  --command 'python train.py --config configs/train.yaml'
rhost job status gpu training --json
rhost job logs gpu training --json --since 0
```

File operations and interactive sessions remain explicit:

```bash
rhost fs read gpu '~/work/project/main.go' --json
rhost fs write gpu '~/work/project/main.go' --from ./main.go --if-hash <sha256>

rhost session create gpu --name debug --cwd '~/work/project'
rhost session exec gpu debug --command 'python -m pdb app.py' --json
```

Creating a file needs no hash; replacing or patching one requires the SHA-256
from the last `fs read`. Sync and mirror delete only with explicit `--delete`.
Remote search stays an ordinary command:

```bash
rhost exec gpu --cwd '~/work/project' --command 'rg TODO src'
```

## What rhost does not own

rhost does not manage SSH keys, maintain a host database, install a remote
agent, run a daemon, or define a deployment language.

OpenSSH owns connections and authentication. Remote tmux owns persistent
terminals. Remote processes own jobs. The filesystem owns files. rhost owns the
contract between those components and the caller.

## Structured results

Every agent-visible behavior has a `--json` path. JSON mode writes exactly one
versioned document to stdout, never mixes progress or command output into it,
and provides stable `error.code` values. Its execution data includes the remote
exit code, stdout and stderr, byte counts, truncation flags, duration,
timeout/cancellation state, and whether cleanup was confirmed.

| Process status | Meaning |
|---|---|
| 0–254 | Remote command status |
| 124 | rhost execution deadline |
| 130 / 143 | rhost handled SIGINT / SIGTERM |
| 255 | Adapter, transport, usage, or local output failure |

A remote program can return the same numeric values. In ambiguous cases inspect
`ok`, `data.exit_code`, and `error.code` instead of branching on English text.
The process-status table does not describe the JSON sentinel: an exec
`data.exit_code` of `-1` means rhost did not observe a remote command status.
Use `error.code`, `data.timed_out`, and `data.cancelled` to classify that result.

## Use as an agent skill

The repository includes a concise skill that teaches an agent to prefer normal
commands and select jobs, sessions, file operations, or tunnels only when their
extra guarantees are needed:

```bash
npx skills add starfield17/rhost --skill rhost
```

The canonical skill is also listed on
[skills.sh](https://skills.sh/starfield17/rhost/rhost). Codex users can install
the same directory with the built-in skill installer:

```text
$skill-installer install the rhost skill from
https://github.com/starfield17/rhost/tree/main/skills/rhost
```

The repository also carries a portable Agent Plugin manifest. The plugin
packages the same skill and requires the `rhost` CLI to be installed separately.

## Design and verification

The CLI process owns no durable remote work. Killing and restarting it does not
erase state it promised to preserve.

- [Project overview](docs/PROJECT_OVERVIEW.md)
- [Architecture and ownership](docs/ARCHITECTURE.md)
- [Agent skill](skills/rhost/SKILL.md)
- [JSON envelope schema](schemas/result-v1.schema.json)
- [Contributor rules](AGENTS.md)

```bash
make check

RHOST_TEST_HOST=<user>@<host> make test-live
RHOST_TEST_HOST=<user>@<host> make test-live-session
RHOST_TEST_HOST=<user>@<host> make test-live-all
```

Live suites always take their target from `RHOST_TEST_HOST`; the repository has
no machine-specific default. See the architecture documents for the exact
runtime, persistence, file, and recovery contracts.
