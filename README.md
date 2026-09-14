# rhost

rhost gives coding agents a local-like command model on any SSH-reachable host.
It delegates target resolution, authentication, host keys, ProxyJump, and
connection reuse to the system OpenSSH client; no remote rhost daemon is
installed.

Rust v4 targets the versioned schema-v2 envelope. It implements foreground
execution, diagnosis, file operations, persistent tmux sessions, dedicated
OpenSSH tunnels, and a local audit trail. `session attach` is deliberately
refused with `USAGE_ERROR` because v2 has no honest terminal-attachment result.

## Install

Install the newest published release for macOS or Linux on amd64 or arm64:

```bash
curl -fsSL https://raw.githubusercontent.com/starfield17/rhost/main/scripts/install.sh | bash
```

The installer verifies the release's SHA-256 file before atomically replacing
an existing binary. Set `RHOST_INSTALL_DIR` to choose another local destination,
or `RHOST_VERSION` to pin a published version. An unqualified install selects the
newest published release.

Build from source with Rust 1.85 or newer:

```bash
make build
install -m 755 target/debug/rhost ~/.local/bin/rhost
```

## Use

```bash
rhost exec <host> --cwd '<remote-directory>' --command 'git status --short'
rhost doctor <host> --json
rhost fs read <host> '<remote-file>' --json
rhost session create <host> --name debug --json
rhost tunnel list --json
```

The command value is one program interpreted by a remote login bash. Human mode
preserves ordinary stdin/stdout/stderr behavior. Use `--json` for orchestration
and branch on `error.code` plus typed evidence under `data.execution`,
`data.cleanup`, and `data.output`; never parse English diagnostics.

Foreground exec has no default deadline. Persistent connections belong to
OpenSSH, sessions to remote tmux/state, and tunnels to dedicated OpenSSH masters
plus local records. The CLI process owns no durable state it promises to keep.

## Agent skill and plugin

Install the repository skill with a compatible skill installer:

```bash
npx skills add starfield17/rhost --skill rhost
```

Codex users can also install `skills/rhost` from this repository. The root
`plugin.json` is the portable Agent Plugin entry point;
`.codex-plugin/plugin.json` exposes the same skill to current Codex plugin
discovery. Both require the `rhost` CLI to be installed separately.

## Development and status

```bash
make test-smoke
make check
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-all
# Optional frozen historical comparison (requires Go):
RHOST_BIN=./target/debug/rhost make test-conformance
```

`test-smoke` is the complete local Rust black-box suite: it uses denying tool
stubs and never contacts a host. `test-live-all` is the serial, Rust-native real
SSH release gate. The frozen Go harness remains available only through
`test-conformance` and the explicitly named `test-legacy-live-*` targets.

[`docs/CONTRACT.md`](docs/CONTRACT.md) is the semantic ledger and
[`schemas/result-v2.schema.json`](schemas/result-v2.schema.json) is the Rust wire
target. See [`docs/RUST_MIGRATION.md`](docs/RUST_MIGRATION.md) for the exact
implemented-versus-verified state. The frozen Go v3.1.0 implementation and its
historical v1 contract remain read-only under `archive/`.
