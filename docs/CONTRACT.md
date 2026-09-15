# rhost contract ledger

This ledger is the migration authority for product semantics. The Go reference
is v3.1.0 (`d9daaa9`); the Rust candidate targets v4 with wire schema v2.
Language replacement does not grant permission to reinterpret execution evidence,
retryability, resource ownership, or destructive operations.

## Authority and evidence

- [Architecture](ARCHITECTURE.md) explains the guarantees; this ledger identifies
  them. [v1](../archive/go-v3.1.0/schemas/result-v1.schema.json) remains immutable wire history;
  [v2](../schemas/result-v2.schema.json) defines the candidate's representation.
- The owner reports real SSH verification of Go v3.1.0. That is baseline evidence,
  not evidence for the extracted harness. The Rust candidate's complete native
  live suite passes against a real remote Linux host over SSH in this revision.
- `make check` verifies the local implementation, schema, frozen corpus hashes,
  Rust foundation, and the map from all 27 frozen live names to native Rust
  evidence. The extracted live corpus remains an optional historical comparison.
  A live result requires an explicit `RHOST_TEST_HOST`; the recorded full-suite
  result used an explicit target and candidate binary.
- Links to Go unit tests are implementation evidence, not independent candidate
  conformance. A mechanism-specific test is not a requirement to copy that mechanism.
- Where Go and this ledger intentionally differ, the difference is listed below.
  An unlisted conflict must be recorded in [FRICTION.md](../FRICTION.md) before
  changing a contract or acceptance test.

The JSON paths below are v2 paths. A dash means a CLI/environment guarantee whose
observable outcome is in the associated operation envelope, not a new JSON field.
Test symbols are stable pointers within the linked files.

## Product and execution

Source: [runtime](architecture/runtime.md), [product rules](ARCHITECTURE.md#product-rules).

| ID | Required behavior | JSON path | Executable evidence |
| --- | --- | --- | --- |
| EXEC-001 | Exactly one shell program crosses the remote boundary; do not rebuild it from argv fragments. Preserve stdin and independent human streams. | `data.output` | [TestLiveExec](../archive/conformance-v1/conformance/live_test.go); [CLI input tests](../archive/go-v3.1.0/internal/cli/command_input_test.go) |
| EXEC-002 | No default deadline for foreground exec. Explicit timeouts bound the local invocation. | `error.code` | [CLI tests](../archive/go-v3.1.0/internal/cli/root_test.go); [TestLiveExecTimeout](../archive/conformance-v1/conformance/live_test.go) |
| EXEC-003 | A remote nonzero exit is completed execution, not adapter failure; return that status locally. | `ok`, `data.execution.exit_code` | [TestLiveExec](../archive/conformance-v1/conformance/live_test.go); [remote_nonzero_is_completed_and_delivery_failure_preserves_evidence](../tests/domain.rs) |
| EXEC-004 | Completion requires evidence bound to this invocation and a valid exit code. | `data.execution` | [VerifiedCompletion](../src/domain/identity.rs); [protocol regressions](../archive/go-v3.1.0/internal/transport/openssh/protocol_test.go) |
| EXEC-005 | Missing completion evidence never becomes known success or an assumed connectivity failure. | `data.execution.status`, `error.code` | [TestConformanceExecUncertainty](../archive/conformance-v1/conformance/hermetic_test.go); [TestSchemaV2RejectsFalseEvidence](../archive/conformance-v1/conformance/schema_test.go) |
| EXEC-006 | Timeout/cancellation with possible effects is non-retryable, including confirmed cleanup. | `error.retryable`, `data.cleanup.status` | [cleanup_is_not_permission_to_retry](../tests/domain.rs); [TestLiveExecDirectCancellation](../archive/conformance-v1/conformance/live_test.go) |
| EXEC-007 | Cleanup confirms only the matched managed process group stopped, never that effects or detached work did not occur. | `data.cleanup.status` | [cleanup tests](../archive/go-v3.1.0/internal/app/exec_stream_test.go); [TestLiveExecTimeout](../archive/conformance-v1/conformance/live_test.go) |
| EXEC-008 | Foreground completion and local interruption are independent: retain a known exit after timeout/cancellation or inherited-pipe failure. | `data.execution`, `error.code` | [completion_and_interruption_are_independent](../tests/domain.rs); [native live exec races](../tests/live/exec.rs); [TestCompletedForegroundSurvivesInheritedBackgroundPipe](../archive/go-v3.1.0/internal/app/exec_stream_test.go) |
| EXEC-009 | Bound JSON capture independently of live forwarding; counts describe source bytes. UTF-8 replacement must not alter counts. | `data.output.*` | [TestLiveTools](../archive/conformance-v1/conformance/live_tools_test.go); [invalid_utf8_does_not_corrupt_source_byte_evidence](../tests/domain.rs) |
| EXEC-010 | Output delivery failure is non-retryable; never rerun the operation or restart a partial JSON document. | `error.code` when deliverable; process status and stderr otherwise | [TestConformanceOutputDeliveryFailure](../archive/conformance-v1/conformance/hermetic_test.go); [failed_sink_is_not_retried](../tests/domain.rs) |

## Wire and CLI

Source: [files and JSON](architecture/files-and-json.md).

| ID | Required behavior | JSON path | Executable evidence |
| --- | --- | --- | --- |
| WIRE-001 | Binary and schema versions are independent. Go emits v1; Rust emits v2 with a closed current operation set and no job surface. | `schema_version`, `operation` | [TestConformanceCLI](../archive/conformance-v1/conformance/contract_test.go); [TestSchemaV2Fixtures](../archive/conformance-v1/conformance/schema_test.go) |
| WIRE-002 | JSON stdout contains one document and no progress/prose. `--json --stream` mirrors live streams to stderr. | Entire envelope | [CLI stream tests](../archive/go-v3.1.0/internal/cli/root_test.go); [validated envelope decoder](../archive/conformance-v1/conformance/contract_test.go) |
| WIRE-003 | `ok` describes adapter completion. Consumers branch on stable `error.code`, not English diagnostics or numeric exit alone. | `ok`, `error.code`, `error.retryable` | [TestConformanceExecUncertainty](../archive/conformance-v1/conformance/hermetic_test.go); [schema tests](../archive/conformance-v1/conformance/schema_test.go) |
| WIRE-004 | Remote status passes through; timeout is 124, SIGINT/SIGTERM cancellation 130/143, other adapter failure 255. These overlap possible remote statuses. | `data.execution.exit_code`, `error.code` | [TestLiveExec](../archive/conformance-v1/conformance/live_test.go); [domain outcome tests](../tests/domain.rs) |
| WIRE-005 | Unknown exit has no integer in v2. Missing/null data must not silently decode to zero/false. | `data.execution.status` | [TestSchemaV2RejectsFalseEvidence](../archive/conformance-v1/conformance/schema_test.go); [strict field access](../archive/conformance-v1/conformance/contract_test.go) |
| WIRE-006 | Empty command is `USAGE_ERROR`; whitespace-only command is `CONFIG_INVALID`. Invalid input fails before SSH. | `error.code` | [TestConformanceCLI/command-validation](../archive/conformance-v1/conformance/contract_test.go) |
| AUDIT-001 | Audit is local, bounded and fail-open; never log environment maps, file contents or send payloads. | `data.entries[]` | [TestLiveAudit](../archive/conformance-v1/conformance/live_audit_test.go); [audit unit tests](../archive/go-v3.1.0/internal/audit/audit_test.go) |

## Persistent state and sessions

Source: [persistent work](architecture/persistent-work.md).

| ID | Required behavior | JSON path | Executable evidence |
| --- | --- | --- | --- |
| PERSIST-001 | Durable connections belong to OpenSSH; sessions to remote tmux/state; tunnels to dedicated masters and local records. No CLI-owned durable map or daemon. | `connection.status`, `session.list`, `tunnel.list` results | [TestLiveTransportReuse](../archive/conformance-v1/conformance/live_test.go); [TestLiveSession](../archive/conformance-v1/conformance/live_session_test.go); [TestLiveTunnelPersistence](../archive/conformance-v1/conformance/live_tools_test.go) |
| PERSIST-002 | v4 does not adopt or delete v3 state. Upgrade requires explicitly closing old sessions/tunnels before switching; v4 uses a separate namespace. An explicit `RHOST_REMOTE_STATE` is the complete remote v4 root used consistently by exec cleanup, doctor and sessions. | `config::v4_state_dir`, `RHOST_STATE_DIR/v4` (audit trail), `${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost/v4}` | **Implemented**; [migration policy](RUST_MIGRATION.md#state-and-upgrade-policy) |
| SESSION-001 | Freeze create/list/exec/send/read/attach/recover/close. Do not introduce scheduling, windows or layout management. | `operation` | [v2 schema](../schemas/result-v2.schema.json); [TestConformanceCLI](../archive/conformance-v1/conformance/contract_test.go) |
| SESSION-002 | Preserve cwd, environment, shell process and discovery across CLI exits. | `data.sessions[]` | [TestLiveSession](../archive/conformance-v1/conformance/live_session_test.go) |
| SESSION-003 | Exec writes only when the idle managed shell owns the pane. A foreground REPL returns `SESSION_BUSY` without receiving command input. | `data.execution.status`, `error.code` | [TestLiveSessionPythonRecovery](../archive/conformance-v1/conformance/live_session_protocol_test.go); [TestLiveSessionBusyRefusesExec](../archive/conformance-v1/conformance/live_session_test.go) |
| SESSION-004 | All writers share the remote lock; paste plus Enter is one submission with invocation-local buffers. Lock contention is retryable `SESSION_UNHEALTHY`, with no submission. | `error.code`, `error.retryable` | [TestLiveSessionConcurrentInputIsolation](../archive/conformance-v1/conformance/live_session_protocol_test.go); [writer protocol tests](../archive/go-v3.1.0/internal/session/tmux/tmux_test.go) |
| SESSION-005 | Send data is verbatim. Read cursors count raw bytes and do not omit/interleave observation intervals. | `data.content`, `data.from`, `data.next`, `data.more` | [TestLiveSessionSendRawInput](../archive/conformance-v1/conformance/live_session_test.go); [cursor/scan tests](../archive/go-v3.1.0/internal/session/tmux/tmux_test.go) |
| SESSION-006 | Completion is invocation-bound. Missing, duplicate, malformed or contradictory evidence yields `SESSION_UNHEALTHY`, not empty success. | `data.execution`, `error.code` | [TestParseExecRejectsIncompleteOrForeignResults](../archive/go-v3.1.0/internal/session/tmux/tmux_test.go); [VerifiedCompletion](../src/domain/identity.rs) |
| SESSION-007 | Canonical ID never becomes the caller's name. Unresolved identity is null in v2; caller reference remains separate. | `data.session_id`, `data.session_ref` | [TestLiveSession](../archive/conformance-v1/conformance/live_session_test.go); [busy_session_has_no_exit_and_no_stdout_alias](../tests/domain.rs) |
| SESSION-008 | PTY output is merged terminal content, never independent stdout/stderr. | `data.output.kind == pty`, `data.output.content` | [Rust DTO schema validation](../archive/conformance-v1/architecture/serialization_test.go); [live session tests](../archive/conformance-v1/conformance/live_session_test.go) |
| SESSION-009 | Recover sends interrupt under the writer lock and requires a fresh shell prompt. Never type an exit command into a REPL. Preserved means usable managed shell, not merely surviving tmux. | `data.session_preserved`, `data.foreground` | [TestLiveSessionPythonRecovery](../archive/conformance-v1/conformance/live_session_protocol_test.go); [TestLiveSessionRecoveryExplicit](../archive/conformance-v1/conformance/live_tools_test.go) |
| SESSION-010 | A create that reached the host reserves its candidate identity before submitting and reports it with `creation_status` on failure: `not_created` (explicit refusal), `unknown` (timeout/cancellation/disconnect/missing evidence), or `created` (creation evidence without a full record). A local mistake or ungeneratable id reports `data: null`. A resolved `--cwd` is recorded as an absolute path and an old record falls back to its metadata literal. | `data.session_id`, `data.session_ref`, `data.creation_status`, `data.initial_cwd` | [Rust create tests](../src/app/session/create.rs); [failure-shape schema cases](../examples/contract_cases.rs) |
| TUNNEL-001 | Alive means the forward exists, not application health. Close only the named dedicated master and its record. | `data.tunnel_id`, `data.tunnels[]` | [TestLiveTunnelPersistence](../archive/conformance-v1/conformance/live_tools_test.go); [tunnel lifecycle tests](../archive/go-v3.1.0/internal/transport/openssh/tunnel_lifecycle_test.go) |

## Files and environment

Source: [files](architecture/files-and-json.md), [runtime](architecture/runtime.md),
[engineering](architecture/engineering.md).

| ID | Required behavior | JSON path | Executable evidence |
| --- | --- | --- | --- |
| SSH-001 | System OpenSSH owns resolution, authentication and host keys. Never weaken checking or install remote packages. | `error.code`, `data.capabilities` | [TestConformanceExecUncertainty](../archive/conformance-v1/conformance/hermetic_test.go); [OpenSSH option tests](../archive/go-v3.1.0/internal/transport/openssh/client_test.go) |
| SSH-002 | Shared-master state is observable; reset stops new channels without killing accepted channels. Fresh mode never initializes shared state. | `data.master_status`, `data.stopped` | [connection tests](../archive/go-v3.1.0/internal/transport/openssh/client_test.go); [CLI recovery tests](../archive/go-v3.1.0/internal/cli/root_test.go) |
| SSH-004 | `doctor` reports the binding each capability resolved to in this execution environment, under the same key set as `capabilities`, or null when absent. A failed probe reports both maps empty; rhost probes no sudo, group, full PATH, version or package ownership. | `data.capability_paths` | [capability-map unit tests](../src/app/doctor.rs); [probe acceptance tests](../tests/acceptance/transport.rs) |
| SSH-003 | Deep or space-containing cache paths must not break transport reuse. The hash/path algorithm is not frozen. | `data.control_path`, `data.master_status` | [TestLiveControlPathDeepCacheDir](../archive/conformance-v1/conformance/live_test.go); [TestLiveFsSyncWithSpaceInCacheRoot](../archive/conformance-v1/conformance/live_fs_test.go) |
| FS-001 | A remote path is one literal value; expand leading home shorthand remotely, reject paired outer quotes before any remote action. | `error.code`, `data.path` | [path tests](../archive/go-v3.1.0/internal/fileops/rsync_paths_test.go); [identities_paths_and_hashes_are_validated](../tests/domain.rs) |
| FS-002 | Plain put/get use scp; resume/checksum use rsync and verify digest. Parent creation is explicit. | `data.backend`, `data.checksum_verified` | [TestLiveFsPutGet](../archive/conformance-v1/conformance/live_fs_test.go); [TestLiveTools](../archive/conformance-v1/conformance/live_tools_test.go) |
| FS-003 | Read is bounded UTF-8 text with SHA-256 of the complete file. | `data.content`, `data.sha256`, `data.truncated` | [TestLiveTools](../archive/conformance-v1/conformance/live_tools_test.go); [remote helper suite](../archive/go-v3.1.0/internal/fileops/remote_test.py) |
| FS-004 | Existing content replacement and patch require matching hash; replacement is locked, same-directory, atomic and preserves permissions. | `error.code`, `data.sha256` | [TestSmokeLive](../archive/conformance-v1/conformance/live_smoke_test.go); [FileWrite](../src/domain/files.rs); [remote helper suite](../archive/go-v3.1.0/internal/fileops/remote_test.py) |
| FS-005 | Sync/mirror never delete without explicit delete; preserve dangerous-path refusal and timeout process cleanup. | `data.delete`, `data.changes`, `error.code` | [TestLiveFsSyncPlanAndApply](../archive/conformance-v1/conformance/live_fs_test.go); [fileops tests](../archive/go-v3.1.0/internal/fileops/sync_test.go) |
| FS-006 | Batch is ordered serial put/get, not a transaction. Continue after entry failures; completed report has ok=true, failed entries make process status 255. | `data.items[]`, `data.failed` | [batch CLI tests](../archive/go-v3.1.0/internal/cli/tools_test.go) |
| RELEASE-001 | Execute the exact four shipping artifacts natively and verify their checksums before publication. The local installer selects the same assets and verifies SHA-256 before replacement; plugin versions mirror Cargo, the sole binary version authority. | `version` result | [release workflow](../.github/workflows/release.yml); [release contract check](../scripts/check-release-contract.sh); [installer test](../scripts/test-install.sh) |

## Mechanisms deliberately not frozen

Internal `RHOST_*` line spelling, wrapper encoding, socket hashing, helper source
language and Go package layout may change. Semantic completion evidence,
canonical identity, lock ownership, incremental reads, cancellation, CAS and
OpenSSH security must not change with them. Existing live test stubs must not be
made to parse a Rust implementation's private marker protocol.

## Explicit v2 differences

- Version 2 removes old job variants and duplicate tunnel `id` fields. Version
  metadata no longer exposes the implementation-specific `go_version` field.
- Exec/session exec use `execution` and typed `output`; session exec no longer
  calls PTY content `stdout`. Exec cleanup has three explicit evidence states.
- Missing session identity is null, never a caller name or empty canonical ID.
- Go reports `INTERNAL` when SSH exits zero without any completion marker; v2
  reports `REMOTE_EXECUTION_UNKNOWN`. Both refuse success and blind retry.
- v2 retains known foreground completion on an output sink failure as well as
  timeout/cancellation. This describes foreground status, not successful delivery.
- Wire v2 and new state namespaces do not imply a compatibility reader or any
  currently implemented remote Rust functionality.
- The Go binary's Cobra-generated shell completion command is not part of the
  agent-facing v2 operation set and is intentionally not reimplemented.

The historical regression index is [history.json](../tests/fixtures/history.json).
