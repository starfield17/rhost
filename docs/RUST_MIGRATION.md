# Rust v4 migration

> Migration completed at v4.1.0. This document is frozen as historical evidence.
> Current product semantics live in [CONTRACT.md](CONTRACT.md); compatibility,
> toolchain, review and release policy lives in [MAINTENANCE.md](MAINTENANCE.md).

## Implemented

Contract ledger, schema-v2 design, independent compatibility harness, Rust
domain types and explicit wire DTOs. The Rust binary implements:

- `version`, `hosts`, `doctor`, `connection status|reset`;
- `exec`: one exact shell program, streamed or captured, `--fresh`, deadlines and
  cancellation, bounded capture, completion evidence, honest unknown;
- `fs put|get` (scp, `--checksum`/`--resume` verified route, `--parents`),
  `fs sync|mirror` (rsync plans, `--dry-run`, `--exclude`, explicit `--delete`,
  resolved-destination refusal) and `fs batch`;
- `fs read|write|patch`: bounded UTF-8 pages with the whole file's SHA-256, and
  hash-guarded, locked, atomic replacement by an embedded remote helper.
- `tunnel open|list|close`: one forward per dedicated OpenSSH master, records
  under this version's own state namespace (`state/v4/tunnels`), `alive` only
  after that master answered a control request, and `stale` records reported
  rather than deleted.
- `session create|list|exec|send|read|recover|close`: a remote tmux session owned
  by the host, one helper submission per call, writers serialised by the
  session's remote lock, token-bound completion for `exec`, byte cursors for
  `read`, and `SESSION_BUSY` instead of typing into a program that owns the pane.
  `session attach` is refused with `USAGE_ERROR` because it needs a terminal and
  the v2 envelope has nowhere honest to put it.
- `audit`: the local JSON Lines trail of remote operations, written fail-open by
  every operation that acts on a host, with `--limit`/`--host` views over it and
  `RHOST_AUDIT=0` to turn it off.

Every operation in the v2 schema is implemented except `session attach`, which
the schema itself requires to be a usage error.

The repository also distributes the v2-aware `skills/rhost` package through a
portable root manifest and the current Codex plugin manifest. The local release
installer maps the four native assets, verifies their SHA-256 files, and replaces
an existing binary only after verification succeeds.

The Go v3.1.0 tracked tree from d9daaa9 is frozen byte-for-byte under
archive/go-v3.1.0, including VERSION, plugin.json, go.mod, go.sum, original tests,
skills, installers and release workflows. archive/conformance-v1 contains the
extracted independent harness and historical corpus, finalized before freeze.
Do not change either snapshot or its integrity manifest. Future work is Rust only.

Cargo.toml is the sole active version authority; schema version 2 is independent
and does not track the binary version. The root build/check/CI use Rust. Go is
optional only for running the frozen reference or compatibility harness.

## Checks

```sh
make build-release
make check
make build-reference
RHOST_BIN=./bin/rhost-go make test-conformance
```

make check runs fmt, clippy, Rust tests, independent JSON Schema validation of
38 actual DTO cases, archive hashes, the domain boundary guard, standalone domain
compilation, portability, the structure budget (file size and dependency
direction), agent-package validation, hermetic installer tests and the release
contract check. A version tag builds, executes and verifies the four shipping
artifacts natively, then publishes only after all four succeed. CI additionally
rejects archive changes against the base commit once the freeze exists there.

The verification stack is now Rust-first and deliberately layered:

- `make test-smoke` runs the complete local black-box acceptance crate with
  denying `ssh`/`scp`/`rsync` defaults. It needs no configured host and cannot
  fall through to a developer's real SSH configuration. It currently contains
  62 cases and passes locally.
- Native live tests live in the feature-gated `tests/live_exec.rs` crate and
  `tests/live/` capability modules. `make check` compiles and lints them without
  contacting a host or adding ignored tests. Every runtime answer is validated
  against schema v2 before fields are inspected.
- All native live targets require both `RHOST_TEST_HOST` and `RHOST_BIN`. The
  latter is resolved before the test changes directory, so the suite exercises
  exactly the candidate or release artifact the caller named.
- `make test-live-smoke` is the short real-SSH path and passes against a real
  remote Linux host over SSH in this revision. `make test-live`,
  `test-live-fs`, `test-live-session`, `test-live-tunnel`, and `test-live-audit`
  isolate failures by capability. `make test-live-all` runs every native case
  serially and is the release gate; `test-live-rust` is a compatibility alias.
- `tests/live/legacy-map.tsv` accounts for all 27 frozen live names. The local
  `check-live-suite` gate rejects missing mappings, nonexistent Rust symbols, or
  individually ignored native live tests without executing Go.
- The frozen Go harness remains available as `make test-conformance` and the
  explicitly named `test-legacy-live-*` targets. It is historical comparison
  evidence, not a dependency of the Rust release gate.

The native live crate covers foreground completion and cancellation races,
bounded capture, doctor and ControlMaster reuse/reset, deep cache roots, file
transfer/CAS/sync safety, session persistence/busy/recovery/client death, tunnel
persistence and reverse traffic, and audit persistence/redaction/concurrent
writes. Both the smoke selector and the complete 23-case `make test-live-all`
suite pass against a real remote Linux host over SSH in this revision, with no
failed or ignored native live tests.

## Next implementation order

1. Four native release artifacts.
   The release workflow is held to CONTRACT.md RELEASE-001 mechanically: exactly
   the four native platform/runner pairs, version from Cargo.toml alone, artifacts
   named for version and platform, executed and checksum-verified, no
   cross-compilation, no write near the frozen archive, and every action pinned
   to a commit. Manual dispatch remains a non-publishing rehearsal. A matching
   version tag publishes all eight files only after every native matrix job
   succeeds. Each artifact's `version` result also names the commit and UTC build
   time it was built from, and the contract check fails when that assertion
   leaves the workflow.

Each subsystem needs failing acceptance evidence before implementation. Never
change contracts or weaken tests merely to accommodate candidate behavior.
Go is evidence, not an absolute oracle; intentional v2 differences are recorded
in CONTRACT.md. Historical gaps remain explicitly marked in tests/fixtures/history.json.

## Architecture boundary

One crate. Pure domain values cannot depend on serialization, I/O, environment,
process or network modules. Private outcomes and identity newtypes rule out
invalid combinations; single-use verified completion is required for success.
Output maps domain evidence explicitly to schema v2. No async runtime, SSH
library, hidden daemon or one-implementation trait framework.

Binary version and wire schema may change independently. The authorized migration
does not require persisted Go state compatibility: close old sessions and tunnels
before switching. Do not add dual-state migration machinery.

## State and upgrade policy

v4 keeps its own namespaces. Locally that is `RHOST_STATE_DIR/v4/` (tunnel records
and the audit trail). On the remote host the complete v4 root is
`${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost/v4}`; session records and the
exec wrapper's pid files live beneath it. It never reads, migrates or deletes
what a v3 installation left behind, so upgrading means closing old sessions and tunnels while v3 is still
installed. Sockets are not state and stay in the shared control directory,
because what they hold is OpenSSH's connection reuse, not rhost's bookkeeping.
