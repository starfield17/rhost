# Rust v4 migration

## Implemented bootstrap

Contract ledger, schema-v2 design, independent compatibility harness, Rust
domain types and explicit wire DTOs are present. The Rust binary implements
version/help and skeleton usage errors only. Remote operations are pending.

The Go v3.1.0 tracked tree from d9daaa9 is frozen byte-for-byte under
archive/go-v3.1.0, including VERSION, plugin.json, go.mod, go.sum, original tests,
skills, installers and release workflows. archive/conformance-v1 contains the
extracted independent harness and historical corpus, finalized before freeze.
Do not change either snapshot or its integrity manifest. Future work is Rust only.

Cargo.toml is the sole active version authority (4.0.0-alpha.1); schema version 2
is independent. The root build/check/CI use Rust. Go is optional only for running
the frozen reference or compatibility harness.

## Checks

```sh
make build
make check
make build-reference
RHOST_BIN=./bin/rhost-go make test-conformance
```

make check runs fmt, clippy, Rust tests, independent JSON Schema validation of
35 actual DTO cases, archive hashes, the domain boundary guard, standalone domain
compilation and portability. CI additionally rejects archive changes against
the base commit once the freeze exists there.

Live wrappers require both RHOST_TEST_HOST and RHOST_BIN and retain the original
suite selectors, parallelism and deadlines. No live test has been run in this
bootstrap. The owner reports the original Go binary passed real SSH testing;
this is not evidence for the extracted harness or Rust implementation.

## Next implementation order

1. Rust black-box conformance runner and historical regression scenarios.
2. CLI grammar/errors/output, configuration, hosts and doctor.
3. OpenSSH argv and connection ownership; foreground execution, streaming,
   bounded capture, interruption, descendants holding pipes and honest unknown.
4. Files with CAS, literal paths and bounded transfer lifecycle; tunnels.
5. Sessions last: pane ownership, atomic submission, invocation evidence,
   raw-byte cursors, recovery and cross-process persistence.
6. Full conformance/live verification, then four native release artifacts.
   The current manual workflow is a non-publishing rehearsal only.

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
