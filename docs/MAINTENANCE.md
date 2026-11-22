# Maintenance policy

v6.0 uses one schema-v6 envelope for all public JSON operations. Changes are conservative by default:
the contract and existing gates define the product, and implementation work does
not grant permission to reinterpret them.

## Wire compatibility

Binary and wire-schema versions are independent. `schema_version` names a
compatibility generation, not a release train.

A producer may keep its current schema version only when every DTO shape it can
emit is accepted by the published compatibility validator for that generation.
All actual DTO cases must pass the active schema v6. Published v5.1.0 schema-3
request/result validators and fixtures remain frozen and digest-protected.
Published v5.2.0 schema-4 and v5.3 schema-5 validators and fixtures are also
frozen. Schema 6 adds Run containment and FOLLOW pages. Tunnel results replace
`status` with `master_alive` and `destination_evidence`. Schema 5 adds
bounded file/Run/session observations and executable range continuations;
callers must use its validator and honor explicit coverage and retention gaps.
Schema 4 adds required recovery evidence and replaces search/list shapes; callers
must update their validator and continuation handling. Durable v5 Run and
changeset records remain compatible. Preserved v4 payloads
projected onto their frozen v2 envelope continue to pass the published validator
in `schemas/compat/result-v2-v4.1.0.schema.json`. That baseline is a
byte-for-byte copy from the v4.1.0 tag, protected by a SHA-256 assertion and CI;
it is test input, not a second active schema authority.

The active schema may still be clarified or widened when that condition holds;
the schema, DTO cases, agent documentation and validators change together. If a
new required or emitted closed-object field, enum value, operation, type, null
shape, or removed required field makes the published validator reject an output,
the schema version must increase.

v4.1.0 is the one pre-policy exception: it retained schema v2 while adding
`doctor.data.capability_paths` and the `session.create` partial-failure shape,
which v4.0.x validators can reject. The v4.1.0 validator is therefore the
compatibility baseline for subsequent schema-v2 producers. Do not create a
negotiation mode or tie a schema bump to the binary major version. A schema bump
ships in a binary minor or major release, never a patch.

## Release and toolchain policy

- Patch releases contain fixes and compatible maintenance. Minor releases may
  add capabilities, bump the schema generation, or carry an unavoidable MSRV
  change. Binary majors are reserved for broad CLI or state-ownership breaks.
- Rust 1.85 is the MSRV for the 4.x line. Raise it only when a security or
  dependency constraint makes that necessary, document the reason, and ship the
  change in a minor release.
- Operators can run real-remote suites as optional **manual pre-release**
  verification. CI uses disposable strict-host-key SSH fixtures for Linux
  helper acceptance.
- Releases are event-driven. Ship security or user-visible correctness fixes
  promptly and batch low-risk maintenance. A version tag runs `make check` on
  Linux and macOS and builds and verifies the native release binaries.
- Keep `Cargo.lock`, `--locked`, weekly grouped Dependabot updates and the small
  runtime dependency set. Adding a runtime dependency requires architecture
  review. Do not add another dependency manager or supply-chain tool without a
  concrete need.
- Repository checks and the local release installer require Python 3.11+;
  CI selects that version explicitly. The standalone installer stays a
  single-file `curl | bash` entry whose implementation runs in Python.

## Change risk

Risk is assigned by the behavior changed, with paths providing the default.
Use role names in review policy; particular model names do not belong here.

| Risk | Default area | Required handling |
| --- | --- | --- |
| A | `domain/**`, `docs/CONTRACT.md`, `schemas/**`, process cleanup, transport/session protocols and scripts, exec cancellation, release semantics | Architecture review before implementation; name affected consumers and whether the change is compatible. |
| B | A capability's `ops/**`/`dto`/`run` (exec, doctor, files, session), file-tool execution and tunnel lifecycle | Implementation plus a high-level final review against contract and failure semantics. |
| C | Help prose (`dispatch/usage`), fixtures, `cli` primitives, contract-neutral dispatch wiring, documentation and mechanical refactors | The implementation agent may own the change under existing gates. |

Any change to accepted argv, JSON shape, `error.code` or retryability, durable
state, destructive behavior, cancellation/cleanup, or publication semantics is
Risk A regardless of its path.

## Measurement integrity

`scripts/check-integrity.sh BASE` inspects each non-merge commit in the changed
range and fails when one deletes a test file, drops the `#[test]` or assertion
count, adds an `#[ignore]`, removes a `make check` prerequisite, or narrows a gate
invocation. CI runs it against the pull request or push base; the release
`verify` job runs it against the previous tag. It is heuristic, not a proof: it
catches removal, not a weakened tolerance or a widened timeout. A measurement
that is genuinely wrong is changed by itself — no `src/` in that commit — with
a `GateChange: <reason>` commit trailer. Later implementation commits in the
same release range do not invalidate that separation. A misfire is recorded in
`FRICTION.md`.

## Regression memory and flaky tests

`tests/fixtures/history.json` is the permanent engineering-memory index. A
serious defect first found in real use, CI or live verification is complete only
when its fix has a contract ID, regression evidence and a history row. The Rust
test suite validates those links against active Rust evidence.

For a flaky test: record the first observation without weakening it; on the
second observation create a standalone friction/tracker entry with platform,
elapsed time and process evidence; on the third, require a deterministic
reproducer before unrelated work proceeds. Retries, ignores and wider production
timeouts are not diagnoses. Expensive stress targets stay opt-in rather than in
`make check`.

## Historical migration record

[`RUST_MIGRATION.md`](RUST_MIGRATION.md) records how v4 replaced the Go
implementation and is frozen as history. Current product semantics live in
[`CONTRACT.md`](CONTRACT.md); current maintenance and release decisions live in
this document.


## Native helper and MCP toolchain

The v5 client MSRV is Rust 1.88 for the official MCP SDK. Runtime dependencies
add rmcp with default features disabled and only server/stdio support, plus
Tokio for that adapter. HTTP transports and authentication stacks are disabled.
`mcp` owns SDK types; capabilities return the same typed request outcomes to CLI
and MCP. Cancellation is request-scoped and stops observation, never a Run.

The opt-in live cgroup case uses `RHOST_TEST_CGROUP_ROOT` as the remote
operator-delegated subtree. Without that variable it verifies pre-exec refusal.

Local and CI `make check` build a native C test helper when
`RHOST_HELPER_BIN` is absent. `make test-smoke` uses the same build. The
CI helper matrix builds static musl helpers on both Linux architectures
and a native macOS arm64 test helper. It runs exact-binary acceptance and
disposable SSH on Linux, with a separate Linux ASan/UBSan and allocation-failure
build. The client uses a versioned HTTPS SHA-256 sidecar to verify downloaded
Linux helpers. Automatic preparation can also use an existing remote C99
compiler. A configured client-side compiler is an explicit build choice.

## Publication procedure

1. Verify the version in `Cargo.toml` and its plugin mirrors. Run `make check`
   and `make test-smoke`. If using a real remote host, set explicit
   `RHOST_TEST_HOST`, `RHOST_BIN` and `RHOST_HELPER_BIN` for `make test-live-all`
   and `make test-live-helper`.
2. Optionally dispatch `release.yml` for a rehearsal. This runs source checks,
   native builds, and helper verification and retains their artifacts.
3. Create and push the annotated tag `v<version>` on the verified commit.
   The tag workflow runs the full source gate and compares measurement
   integrity with the previous release. It builds and executes all four native
   clients and verifies the helpers built from the same checkout.
4. Once every verification job passes, publication checks the six expected
   binaries and their SHA-256 sidecars from that workflow run and publishes
   them to GitHub Releases. Keep a published tag fixed.
