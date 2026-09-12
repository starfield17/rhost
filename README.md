# rhost

Run an ordinary shell command on an SSH-reachable host with the same input,
output, and exit-code shape as a local command.

```bash
rhost --host gpu -- 'ls -a'
rhost --host gpu --cwd '~/work/project' -- 'pytest -q'
printf 'hello\n' | rhost --host gpu -- 'cat'
rhost --host gpu --json -- 'git status --porcelain'
```

The command after `--` is one complete shell string. It runs in a fresh remote
login-bash context, so pipelines, redirects, variables, and compound shell syntax
work on the remote host. `--cwd` names a remote directory and `--env KEY=VALUE`
may be repeated.

Human mode forwards stdin and streams stdout/stderr as they arrive. It has no
default execution deadline or output cap. `--timeout 30s` adds a deadline.
`--json` returns one versioned result envelope and captures at most 1 MiB per
stream by default; use `--max-output-bytes 0` for unlimited capture.

Hosts are named exactly as for `ssh`: an OpenSSH config alias, `user@host`, or a
bare hostname. OpenSSH remains responsible for configuration, authentication,
host keys, ProxyJump, and connection reuse.

## When the command needs more than foreground execution

The direct form is the default. The remaining subcommands exist where a plain
foreground command cannot cheaply provide the required guarantee:

- `job`: start a detached command that survives this CLI and the SSH connection;
- `session`: keep an interactive shell or REPL in remote tmux;
- `fs`: transfer files, synchronize directories, and perform hash-protected text edits;
- `tunnel`: create a persistent OpenSSH forward;
- `doctor`: diagnose remote dependencies and connection problems;
- `hosts`: list discoverable OpenSSH aliases;
- `audit`: inspect the local operation trail;
- `version`: report build identity.

The older `rhost exec <host> -- <command...>` form remains available for
compatibility. It keeps its 60-second and 1 MiB defaults. New integrations should
use the direct form because its single command string has explicit shell
semantics and its ordinary output behaves like a command-line process.

## Structured results and exit codes

Every agent-visible result has a `--json` path. JSON mode writes exactly one
document to stdout and never mixes command output into it. Its `data` includes
the remote exit code, stdout/stderr, byte counts, truncation flags, duration,
timeout/cancellation state, and whether remote cleanup was confirmed.

| process status | meaning |
|---|---|
| 0–254 | the remote command's status |
| 124 | rhost execution deadline |
| 130 / 143 | rhost handled SIGINT / SIGTERM |
| 255 | adapter, transport, usage, or local output failure |

A remote program can return the same numeric values. In ambiguous cases inspect
`ok`, `data.exit_code`, and `error.code` in JSON rather than guessing from the
process status.

## Files, jobs, and sessions

```bash
rhost fs put gpu ./file '~/work/file'
rhost fs put gpu ./file '~/new/project/file' --parents
rhost fs get gpu '~/work/result.json' ./result.json
rhost fs sync gpu ./project '~/work/project' --dry-run
rhost fs read gpu '~/work/project/main.go' --json
rhost fs write gpu '~/new/project/main.go' --from ./main.go --parents
rhost fs write gpu '~/work/project/main.go' --from ./main.go --if-hash <sha256>

rhost job start gpu --json --cwd '~/work/project' -- 'make long-task'
rhost job status gpu <job-id> --json
rhost job logs gpu <job-id> --json --since 0

rhost session create gpu --name debug --cwd '~/work/project'
rhost session exec gpu debug --json -- 'python -m pdb app.py'
rhost session send gpu debug --data 'next()' --enter
rhost session read gpu debug --json --since 0
```

Creating a file needs no hash; replacing or patching one requires the SHA-256
from the last `fs read`. Writes use atomic same-directory replacement and
reject symlink targets. Transfers and writes create missing directories only
with `--parents`. Sync and mirror
delete only with explicit `--delete`; preview destructive syncs with `--dry-run`.
Remote search is intentionally an ordinary command, for example:

```bash
rhost --host gpu --cwd '~/work/project' -- 'rg TODO src'
```

## Architecture

The CLI process owns no durable remote work. OpenSSH ControlMaster owns transport
reuse, remote tmux owns sessions, and detached remote processes plus remote files
own jobs. Killing and restarting `rhost` therefore does not erase state it
promised to preserve.

- [Project overview](docs/PROJECT_OVERVIEW.md)
- [Architecture map](docs/ARCHITECTURE.md)
- [Agent skill](skills/rhost/SKILL.md)
- [JSON envelope schema](schemas/result-v1.schema.json)
- [Contributor rules](AGENTS.md)

## Build and verification

```bash
make build
make check

RHOST_TEST_HOST=<user>@<host> make test-live-smoke
RHOST_TEST_HOST=<user>@<host> make test-live
RHOST_TEST_HOST=<user>@<host> make test-live-session
RHOST_TEST_HOST=<user>@<host> make test-live-all
```

Live suites always take their target from `RHOST_TEST_HOST`; the repository has
no machine-specific default. The remote side is an SSH-reachable Linux host and
the local side is macOS or Linux. Use `test-live-smoke` for frequent checks of
the main exec, file, session and job workflows; `test-live-all` remains the full
failure, persistence and transport gate.
