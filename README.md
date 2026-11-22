# rhost

Operate SSH-reachable Linux hosts from coding agents or scripts over standard OpenSSH—covering foreground execution, recoverable background work, compare-and-swap file edits, and persistent tmux sessions. One capability layer backs the CLI, single-document JSON requests (`schema_version: 6`), and stdio MCP tools, with zero remote daemon and zero remote Python.

```text
Codex / Claude / CLI / JSON (schema v6)
  -> local OpenSSH (ControlMaster, ~/.ssh/config, host keys, ssh-agent)
     -> C99 remote helper / tmux (per-Run monitors, CAS files, PTY sessions; no daemon, no Python)
```

## Why it's built this way

- **OpenSSH instead of a custom SSH stack**: Reuses `~/.ssh/config`, `ssh-agent`, host-key verification, and `ControlMaster` (`ControlPersist=15m`). `rhost` never prompts for credentials, stores keys, or weakens host-key checks.
- **C99 helper instead of remote Python**: File operations and recoverable Runs run through a standalone C99 binary (`helper/`) cached privately on the host (`~/.cache/rhost/helper/`), with durable Run/changeset state under `~/.local/state/rhost/v5` (`RHOST_REMOTE_STATE`). Preparation reuses a valid cache, compiles with an existing remote C99 `cc`, or uploads a static Linux musl release binary verified against its HTTPS SHA-256 sidecar (`linux-x86_64`, `linux-aarch64`), with no remote daemon and no automatic package installs.
- **Bounded observation and recoverable Runs instead of blind SSH retries**: File reads default to **8 KiB** (`8192` bytes) and Run/session log previews default to **4 KiB** (`4096` bytes) with source-byte cursors (`run.watch` can follow both streams). Timeouts and client cancellations only end local observation; uncertain `run.start` submissions deduplicate via namespace `generation` and caller-retained expiring keys, and file replacements require a whole-file SHA-256 match.

## Install

Release binaries (macOS and Linux on `amd64`/`arm64`; requires local Python 3.11+, verifies SHA-256 before atomic replacement):

```bash
curl -fsSL https://raw.githubusercontent.com/starfield17/rhost/main/scripts/install.sh | bash
```

Use `RHOST_VERSION` to pin a release or `RHOST_INSTALL_DIR` to set the binary directory (defaults to `~/.local/bin`, or an existing writable standalone `rhost` / Homebrew `bin` on macOS `PATH`). Run `rehash` or `hash -r` if an open shell cached a prior path.

From source (Rust `1.88.0+`):

```bash
git clone https://github.com/starfield17/rhost.git
cd rhost
make build-release
install -m 755 target/release/rhost ~/.local/bin/rhost
```

`make build-release` runs `cargo build --locked --release --bin rhost` and stamps commit/build date into `rhost version --json`; `make build` builds the debug binary.

## Quick start: connect an agent

Ensure noninteractive OpenSSH works first using an SSH alias, `user@example-host`, or hostname:

```bash
codex mcp add rhost -- rhost mcp
claude mcp add --transport stdio --scope local rhost -- rhost mcp
```

Generic stdio clients launch `rhost` with `["mcp"]` (use an absolute path if a GUI client's `PATH` differs). Filter advertised tools at startup with `rhost mcp --tools runs,files` (default `all`; groups: `runs`, `files`, `sessions`, `diagnostics`).

Install the optional skill to load workflow and recovery guidance on demand:

```bash
npx skills add starfield17/rhost --skill rhost
```

[plugin.json](plugin.json) and [.codex-plugin/plugin.json](.codex-plugin/plugin.json) package [skills/rhost/SKILL.md](skills/rhost/SKILL.md); they require the `rhost` CLI separately and do not register MCP. See [MCP setup guide](skills/rhost/references/MCP.md) for client configuration, `SSH_AUTH_SOCK`, tool schemas, and cancellation semantics.

## Operations and examples

| Need | Interface | Reference |
| --- | --- | --- |
| Foreground command, script, stdin, live output | CLI `exec` | [CLI.md](skills/rhost/references/CLI.md#foreground-execution) |
| Recoverable work surviving CLI/MCP exit | CLI, JSON, MCP `run.*` | [RUNS.md](skills/rhost/references/RUNS.md) |
| Bounded read/search, CAS edit, changesets | CLI, JSON, MCP `fs.*` | [FILES.md](skills/rhost/references/FILES.md) |
| Stateful shell, REPL, debugger (`tmux`) | CLI, JSON, MCP `session.*` | [SESSIONS.md](skills/rhost/references/SESSIONS.md) |
| Version, aliases, host diagnosis, master state | CLI, JSON, MCP | [MCP.md](skills/rhost/references/MCP.md#discover-and-call-tools) |
| File copies/sync (`scp`/`rsync`), tunnels, audit | CLI (`fs put/get/sync/mirror`, `tunnel`, `audit`) | [CLI.md](skills/rhost/references/CLI.md) |
| Helper cache/build policy, `error.code` recovery, safety | Policy & recovery tables | [NATIVE.md](skills/rhost/references/NATIVE.md#preparation-policies), [RECOVERY.md](skills/rhost/references/RECOVERY.md), [SAFETY.md](skills/rhost/references/SAFETY.md) |

```bash
# Foreground execution (keep the entire remote program inside --command, or use --command-file)
rhost exec gpu --cwd '~/work/project' --command 'cargo test --locked'

# Diagnose remote shell dependencies and prepare the C99 helper
rhost doctor gpu --json
rhost helper prepare gpu --json

# Single closed JSON request (schema v6; MCP structuredContent uses the same envelope)
rhost request --from - <<'EOF'
{"schema_version":6,"operation":"fs.read","host":"gpu","params":{"path":"~/work/project/config.toml","max_bytes":4096}}
EOF
```

- **Diagnosis vs. preparation**: `rhost doctor` probes shell tools and resolved `command -v` paths (`ok:true` means the probe completed; inspect `capabilities`). Native `fs.*` ops prepare the C99 helper automatically; `rhost helper prepare` returns `generation`, `remote_time_unix`, and helper identity before `run start` (`rhost helper status` reads an existing namespace without creating one).
- **Remote tool requirements**: `exec` and `doctor` require `/bin/sh`, `bash`, `ps`, `base64`, and either `setsid` or Perl with `POSIX::setsid`. `session` requires `bash`, `tmux`, `flock` (`session attach` is refused with `USAGE_ERROR`); resumable/checksum transfers require `rsync`; native `fs.*` and `run.*` require the C99 helper (bootstrapped via POSIX `sh` utilities, no remote Python or `tmux`).
- **Schema, state, and boundaries**: Public JSON uses schema v6 (`v6.0.0`). Update request and result validators to v6. Existing Run and changeset records remain readable. Branch on `error.code` and `error.recovery`, not prose. See [state discovery and retention](skills/rhost/references/FILES.md#state-discovery-and-retention) for `run.list`, `fs.changeset.list`, `state.list`, `state.usage`, and `state.prune`. `rhost` does not manage SSH credentials, system packages, Git branches/worktrees, or job scheduling.

## Docs, schemas, and verification

- [Architecture and ownership](docs/ARCHITECTURE.md)
- [Contract ledger](docs/CONTRACT.md)
- [Context-bounded observation (v5.3.0)](docs/architecture/context-v530.md)
- [Request schema v6](schemas/request-v6.schema.json) and [Result schema v6](schemas/result-v6.schema.json)
- [Agent skill](skills/rhost/SKILL.md)
- [Contributor rules](AGENTS.md) and [Maintenance policy](docs/MAINTENANCE.md)

```bash
make test-smoke   # offline Rust black-box suite (no network)
make check        # fmt, clippy, unit/contract/structure/portability/packaging checks

# Manual pre-release real-SSH suites (require an explicit target)
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-session
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-all
```

`make test-smoke` and `make check` run offline; CI builds the shipping Linux helpers and verifies them against disposable strict-host-key `sshd` fixtures. Real-host `test-live*` and `test-live-helper` runs are optional manual pre-release verification; see [Maintenance policy](docs/MAINTENANCE.md).
