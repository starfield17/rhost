# rhost

**Operate SSH-reachable Linux hosts from a coding agent, through OpenSSH.**

Run an ordinary command, submit recoverable work, inspect and edit files, or drive
a persistent terminal. Use the CLI, one JSON request, or native stdio MCP tools.
OpenSSH owns authentication, host keys, configuration and connection reuse.

```bash
rhost exec gpu --cwd '~/work/project' --command 'cargo test --locked'
```

Foreground exec forwards stdin, streams stdout/stderr, and returns the remote
command's status when completion is known. Recoverable Runs use a small C99
helper and a per-Run monitor. Persistent terminals use remote tmux. Durable
state lives remotely; no global rhost daemon or workspace manager is introduced.

## Install

The release installer requires local Python 3.11+ and supports macOS/Linux on
amd64 and arm64. It checks the published SHA-256 before atomic replacement:

```bash
curl -fsSL https://raw.githubusercontent.com/starfield17/rhost/main/scripts/install.sh | bash
```

Use `RHOST_VERSION` to pin a release and `RHOST_INSTALL_DIR` to choose the binary
directory. The installer checks PATH precedence. On macOS it prefers an existing
writable standalone rhost or suitable Homebrew bin on PATH; otherwise it uses
`~/.local/bin`. An open shell may need `rehash` or `hash -r` after installation.

Build this checkout with Rust 1.88 or newer:

```bash
git clone https://github.com/starfield17/rhost.git
cd rhost
make build-release
install -m 755 target/release/rhost ~/.local/bin/rhost
```

`make build-release` uses locked dependencies and stamps commit/build date into
`rhost version --json`. `make build` provides the debug development binary.
v5.2.0 adds bounded independent MCP control admission, tool groups, structured
recovery evidence, paginated state discovery and durable path collections.
Public JSON moves to schema 4; schema-3 callers must update their validators.
Inspect the connected server's tool list when the installed binary and skill differ.

## Connect an agent

The CLI and MCP server run in the environment that has your OpenSSH setup.
Use an existing SSH alias, `user@example-host`, or hostname. Authentication must
already work noninteractively; rhost retains OpenSSH's host-key policy.

For Codex:

```bash
codex mcp add rhost -- rhost mcp
```

For Claude Code:

```bash
claude mcp add --transport stdio --scope local rhost -- rhost mcp
```

Limit advertised tools at startup with `rhost mcp --tools runs,files`
(default `all`); discovery and invocation use the same filter.

A generic stdio client launches command `rhost` with args `["mcp"]`. Use an
absolute binary path when a GUI client's PATH differs. See the
[MCP setup guide](skills/rhost/references/MCP.md) for configuration examples,
SSH agent environment, tool arguments and cancellation behavior.

The optional skill teaches operation selection and recovery without loading
every workflow into context:

```bash
npx skills add starfield17/rhost --skill rhost
```

Codex's built-in installer can also install
[`skills/rhost`](https://github.com/starfield17/rhost/tree/main/skills/rhost).
The portable `plugin.json` and `.codex-plugin/plugin.json` package that skill;
they require the CLI separately and do not register MCP automatically.

## Choose the operation

| Need | Interface | Details |
| --- | --- | --- |
| Ordinary command, script, stdin or live output | CLI `exec` | [Foreground execution](skills/rhost/references/CLI.md#foreground-execution) |
| Build/test inspectable after CLI or MCP exit | CLI, JSON or MCP Run | [Recoverable work](skills/rhost/references/RUNS.md) |
| Read/search files, CAS edits, named recovery | CLI, JSON or MCP files | [File workflow](skills/rhost/references/FILES.md) |
| Stateful shell, REPL, debugger | CLI, JSON or MCP session | [Terminal workflow](skills/rhost/references/SESSIONS.md) |
| Build info, alias discovery, shell diagnosis, connection status | CLI, JSON or MCP | [MCP tools](skills/rhost/references/MCP.md#discover-and-call-tools) |
| Copy/sync bytes, forward ports, reset connections, query audit | CLI | [CLI reference](skills/rhost/references/CLI.md) |

MCP and JSON requests call the same capability implementations as their CLI
views. Foreground exec's streaming/pipeline behavior remains a CLI interface.
Run execution supplies the structured noninteractive command model in MCP.
`session attach` is deliberately refused with `USAGE_ERROR`.

Doctor diagnoses shell tools:

```bash
rhost doctor gpu --json
rhost exec gpu --command 'uname -a'
```

For doctor, `ok:true` means the probe completed; inspect capabilities before
relying on optional tools. Native file operations automatically prepare their
helper. Use `rhost helper prepare gpu --json` to obtain native readiness,
generation, remote time and helper identity before submitting a Run.
`rhost helper status gpu --json` observes an existing namespace without creating it.

Direct exec needs remote bash, setsid, ps and a base64 decoder. Sessions need
bash, tmux and flock; resumable/checksum-aware transfers need rsync. Native file
operations and Runs need the C99 helper, without remote Python. Preparation can
reuse a validated cache, use explicitly supplied bytes/compiler inputs, compile
with an existing remote cc, or upload a pinned static Linux release artifact.
It never installs packages. See [preparation policies](skills/rhost/references/NATIVE.md#preparation-policies).

## Preserve completion and mutation evidence

All public JSON uses **schema v4**. CLI JSON writes one result to stdout without
mixing command output or progress into it. MCP returns the same versioned
envelope in structuredContent. Branch on error.code and operation-specific data.

For one JSON operation without MCP:

```bash
rhost request --from - <<'EOF'
{"schema_version":4,"operation":"fs.read","host":"gpu","params":{"path":"~/work/project/config.toml","max_bytes":4096}}
EOF
```

Commands and paths are JSON data, avoiding local shell parsing. This is one
operation, not a general workflow/batch language. For CLI shell programs, keep
the complete program inside `--command`, or use `--command-file`; shell syntax
outside the quoted value runs locally.

A Run's wait budget only limits observation. Retain its generation/key/payload
before submission; an uncertain submission can be deduplicated only with the
same valid identity and payload. Session exec timeout has different semantics
and may interrupt the pane. Cancellation of an MCP call does not itself stop a
Run or send a terminal key. See the workflow guides before retrying effects.

Errors include `recovery.stage`, `recovery.submission` and `recovery.action`.
These distinguish preparation from ambiguous submission without parsing prose.
`retry_same_request` requires retained immutable intent; it never restarts lost work.

File replacement and patches require an observed hash. Structured search and
read-many return hashes and bounded observations. `fs.find` discovers paths without
content reads. Search/find accept explicit `exclude_dirs` and return expiring
collection cursors; later search pages read fresh content only for new paths. Named changesets preflight all
files and journal individual changes; they have no global atomic visibility or
automatic rollback. Validation is a separate Run, and recovery is explicit.

`run.list`, `fs.changeset.list` and `state.list` discover retained records in
pages. `state.usage` reports logical/reserved bytes; `state.prune` takes explicit
IDs and defaults to a preview. Only rhost records/logs are managed. See the
[file and state guide](skills/rhost/references/FILES.md#state-discovery-and-retention).

When evidence is missing, completion stays unknown. Confirmed cleanup does not
undo side effects. Consult [recovery](skills/rhost/references/RECOVERY.md) and,
for destructive sync, exposure or privilege changes,
[safety](skills/rhost/references/SAFETY.md).

## Ownership and verification

rhost owns the contract between the caller and existing remote components.
It does not manage SSH credentials, install system packages, create project
workspaces/Git branches, or schedule/restart jobs. Use the host's established
scheduler when those lifecycle guarantees are needed.

- [Architecture and ownership](docs/ARCHITECTURE.md)
- [Contract and executable evidence](docs/CONTRACT.md)
- [Agent skill](skills/rhost/SKILL.md)
- [Result schema](schemas/result-v4.schema.json) and [request schema](schemas/request-v4.schema.json)
- [Contributor rules](AGENTS.md) and [maintenance policy](docs/MAINTENANCE.md)

```bash
make test-smoke
make check

RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-session
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-all
```

`test-smoke` is the complete local Rust black-box suite without network access.
Live suites use explicit environment targets and exercise real SSH behavior.
Real-host verification is a required manual pre-release step.
Pre-release verification also runs `test-live-helper` with an exact remote
helper supplied through RHOST_HELPER_BIN. CI tests exact locally built helpers
through disposable strict-host-key sshd fixtures; it does not compile replacement
helpers. Production publication binds immutable client/helper artifacts to
same-commit manual live evidence. See the maintenance policy for the procedure.
