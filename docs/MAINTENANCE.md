# Maintenance policy

rhost v4 is the production implementation. Changes are conservative by default:
the contract and existing gates define the product, and implementation work does
not grant permission to reinterpret them.

## Wire compatibility

Binary and wire-schema versions are independent. `schema_version` names a
compatibility generation, not a release train.

A producer may keep its current schema version only when every DTO shape it can
emit is accepted by the published compatibility validator for that generation.
All actual DTO cases must pass both the active schema and the published
validator in `schemas/compat/result-v2-v4.1.0.schema.json`. That baseline is a
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
- Real-remote live verification is a required **manual** pre-release step until a
  CI-reachable test host exists. `make test-live-all` takes its target only from
  `RHOST_TEST_HOST`, so the release workflow cannot run it and does not claim to.
  If a reachable host is provisioned, the workflow must record a live result
  bound to the exact tagged commit and make publication depend on it; only then
  is the live suite a release gate rather than a documented expectation.
- Releases are event-driven: ship a security or user-visible correctness fix
  promptly; otherwise batch low-risk maintenance. A tag must re-run `make check`
  on Linux and macOS before any native artifact is built.
- Keep `Cargo.lock`, `--locked`, weekly grouped Dependabot updates and the small
  runtime dependency set. Adding a runtime dependency requires architecture
  review. Do not add another dependency manager or supply-chain tool without a
  concrete need.

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

`scripts/check-integrity.sh BASE` fails when the changed range deletes a test
file, drops the `#[test]` or assertion count, adds an `#[ignore]`, removes a
`make check` prerequisite, or narrows a gate invocation. CI runs it against the
pull request or push base; the release `verify` job runs it against the previous
tag. It is heuristic, not a proof: it catches removal, not a weakened tolerance
or a widened timeout. A measurement that is genuinely wrong is changed by itself
— no `src/` in the same range — with a `GateChange: <reason>` commit trailer, so
the acceptance change is one visible commit rather than a silent side effect of
an implementation fix. A misfire is recorded in `FRICTION.md`.

## Regression memory and flaky tests

`tests/fixtures/history.json` is the permanent engineering-memory index. A
serious defect first found in real use, CI or live verification is complete only
when its fix has a contract ID, regression evidence and a history row. The Rust
test suite validates those links; the frozen Go corpus remains untouched.

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
