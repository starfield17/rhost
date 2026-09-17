# rhost contract ledger

This ledger is the authority for rhost product semantics. The production Rust
implementation is v4 with wire schema v2. The Go v3.1.0 implementation and its
conformance harness were archived out to the read-only `rhost-go-old` repository
at the v4.4.1 split; they are no longer part of this tree and are not maintained.
Maintenance does not grant permission to reinterpret execution evidence,
retryability, resource ownership, or destructive operations.

## Authority and evidence

- [Architecture](ARCHITECTURE.md) explains the guarantees; this ledger identifies
  them. [v2](../schemas/result-v2.schema.json) defines the current representation;
  v1 was frozen wire history and now lives with the archived Go implementation.
- The Rust implementation's complete native live suite has passed against a real
  remote Linux host over SSH; that suite is a required manual pre-release
  verification, not a workflow-enforced gate, because no CI-reachable target is
  provisioned (docs/MAINTENANCE.md).
- `make check` verifies the local implementation, schema, Rust foundation, the
  structure boundary and the map from every frozen live name in
  `tests/live/legacy-map.tsv` to native Rust evidence.
  A live result requires an explicit `RHOST_TEST_HOST`; the recorded full-suite
  result used an explicit target and selected binary.
- Links to Go unit tests are implementation evidence, not independent Rust
  conformance. A mechanism-specific test is not a requirement to copy that mechanism.
- Where Go and this ledger intentionally differ, the difference is listed below.
  An unlisted conflict must be recorded in [FRICTION.md](../FRICTION.md) before
  changing a contract or acceptance test, and the acceptance change then lands
  on its own with a `GateChange:` trailer (docs/MAINTENANCE.md).

The JSON paths below are v2 paths. A dash means a CLI/environment guarantee whose
observable outcome is in the associated operation envelope, not a new JSON field.
Test symbols are stable pointers within the linked files.

## Product and execution

Source: [runtime](architecture/runtime.md), [product rules](ARCHITECTURE.md#product-rules).

| ID | Required behavior | JSON path | Executable evidence |
| --- | --- | --- | --- |
| EXEC-001 | Exactly one shell program crosses the remote boundary; do not rebuild it from argv fragments. Preserve stdin and independent human streams. | `data.output` | [exact program live test](../tests/live/exec.rs:exact_program_streams_cwd_and_remote_status_are_preserved) |
| EXEC-002 | No default deadline for foreground exec. Explicit timeouts bound the local invocation. | `error.code` | [deadline acceptance test](../tests/acceptance/exec.rs:an_explicit_deadline_bounds_the_local_invocation_and_is_not_retryable) |
| EXEC-003 | A remote nonzero exit is completed execution, not adapter failure; return that status locally. | `ok`, `data.execution.exit_code` | [remote_nonzero_is_completed_and_delivery_failure_preserves_evidence](../tests/domain.rs) |
| EXEC-004 | Completion requires evidence bound to this invocation and a valid exit code. | `data.execution` | [VerifiedCompletion](../src/domain/identity.rs) |
| EXEC-005 | Missing completion evidence never becomes known success or an assumed connectivity failure. | `data.execution.status`, `error.code` | [no silent zero](../tests/acceptance/exec.rs:an_unanswered_run_is_never_a_success_and_never_a_silent_zero) |
| EXEC-006 | Timeout/cancellation with possible effects is non-retryable, including confirmed cleanup. | `error.retryable`, `data.cleanup.status` | [cleanup_is_not_permission_to_retry](../tests/domain.rs) |
| EXEC-007 | Cleanup confirms only the matched managed process group stopped, never that effects or detached work did not occur. | `data.cleanup.status` | [cleanup evidence test](../tests/domain.rs:cleanup_is_not_permission_to_retry) |
| EXEC-008 | Foreground completion and local interruption are independent: retain a known exit after timeout/cancellation or inherited-pipe failure. | `data.execution`, `error.code` | [completion_and_interruption_are_independent](../tests/domain.rs); [native live exec races](../tests/live/exec.rs) |
| EXEC-009 | Bound JSON capture independently of live forwarding; counts describe source bytes. UTF-8 replacement must not alter counts. | `data.output.*` | [invalid_utf8_does_not_corrupt_source_byte_evidence](../tests/domain.rs) |
| EXEC-010 | Output delivery failure is non-retryable; never rerun the operation or restart a partial JSON document. | `error.code` when deliverable; process status and stderr otherwise | [failed_sink_is_not_retried](../tests/domain.rs) |

## Wire and CLI

Source: [files and JSON](architecture/files-and-json.md).

| ID | Required behavior | JSON path | Executable evidence |
| --- | --- | --- | --- |
| WIRE-001 | Binary and schema versions are independent. A schema version is a compatibility generation: every output from a same-generation producer must pass that generation's published compatibility validator, or the schema version increases. v4.1.0 is the documented pre-policy v2 exception. | `schema_version`, `operation` | [DTO compatibility tests](../tests/contracts.rs); [maintenance policy](MAINTENANCE.md#wire-compatibility) |
| WIRE-002 | JSON stdout contains one document and no progress/prose. `--json --stream` mirrors live streams to stderr. | Entire envelope | [stream smoke](../tests/acceptance/exec.rs:local_wrapper_smoke_preserves_real_completion_and_streams) |
| WIRE-003 | `ok` describes adapter completion. Consumers branch on stable `error.code`, not English diagnostics or numeric exit alone. | `ok`, `error.code`, `error.retryable` | [usage-error envelope](../tests/acceptance/main.rs:invalid_invocations_are_usage_errors_carried_in_the_envelope) |
| WIRE-004 | Remote status passes through; timeout is 124, SIGINT/SIGTERM cancellation 130/143, other adapter failure 255. These overlap possible remote statuses. | `data.execution.exit_code`, `error.code` | [domain outcome tests](../tests/domain.rs) |
| WIRE-005 | Unknown exit has no integer in v2. Missing/null data must not silently decode to zero/false. | `data.execution.status` | [unknown exit has no integer](../tests/acceptance/exec.rs:an_unanswered_run_is_never_a_success_and_never_a_silent_zero) |
| WIRE-006 | Empty command is `USAGE_ERROR`; whitespace-only command is `CONFIG_INVALID`. Invalid input fails before SSH. | `error.code` | [whitespace is CONFIG_INVALID](../tests/acceptance/main.rs:a_whitespace_command_is_configuration_not_usage_and_never_reaches_ssh) |
| WIRE-007 | Each `--help` page lists the flags its parser accepts, from the same flag specifications. JSON callers receive the corresponding usage operation and `USAGE_ERROR`, never prose on stdout. | `operation`, `error.code` | [help/parser black-box test](../tests/cli.rs); [usage table test](../src/dispatch/usage.rs) |
| AUDIT-001 | Audit is local, bounded and fail-open; never log environment maps, file contents or send payloads. | `data.entries[]` | [audit redaction test](../tests/acceptance/audit.rs:audit_never_records_environment_values) |

## Persistent state and sessions

Source: [persistent work](architecture/persistent-work.md).

| ID | Required behavior | JSON path | Executable evidence |
| --- | --- | --- | --- |
| PERSIST-001 | Durable connections belong to OpenSSH; sessions to remote tmux/state; tunnels to dedicated masters and local records. No CLI-owned durable map or daemon. | `connection.status`, `session.list`, `tunnel.list` results | [process-boundary smoke](../tests/live/smoke.rs:smoke_core_workflows_cross_process_boundaries) |
| PERSIST-002 | v4 does not adopt or delete v3 state. Upgrade requires explicitly closing old sessions/tunnels before switching; v4 uses a separate namespace. An explicit `RHOST_REMOTE_STATE` is the complete remote v4 root used consistently by exec cleanup, doctor and sessions. | `config::v4_state_dir`, `RHOST_STATE_DIR/v4` (audit trail), `${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost/v4}` | **Implemented**; [historical upgrade policy](RUST_MIGRATION.md#state-and-upgrade-policy) |
| SESSION-001 | Freeze create/list/exec/send/read/attach/recover/close. Do not introduce scheduling, windows or layout management. | `operation` | [v2 schema](../schemas/result-v2.schema.json) |
| SESSION-002 | Preserve cwd, environment, shell process and discovery across CLI exits. | `data.sessions[]` | [state survives processes](../tests/live/session.rs:session_state_identity_and_byte_cursor_survive_cli_processes) |
| SESSION-003 | Exec writes only when the idle managed shell owns the pane. A foreground REPL returns `SESSION_BUSY` without receiving command input. | `data.execution.status`, `error.code` | [busy refuses exec](../tests/live/session.rs:busy_program_refuses_exec_and_recover_restores_the_shell) |
| SESSION-004 | All writers share the remote lock; paste plus Enter is one submission with invocation-local buffers. Lock contention is retryable `SESSION_UNHEALTHY`, with no submission. | `error.code`, `error.retryable` | [concurrent writer isolation](../tests/live/session.rs:killing_one_cli_keeps_the_remote_writer_and_session_alive) |
| SESSION-005 | Send data is verbatim. Read cursors count raw bytes and do not omit/interleave observation intervals. | `data.content`, `data.from`, `data.next`, `data.more` | [raw send/cursor](../tests/live/session.rs:raw_send_exit_status_unknown_target_and_recovery_are_explicit) |
| SESSION-006 | Completion is invocation-bound. Missing, duplicate, malformed or contradictory evidence yields `SESSION_UNHEALTHY`, not empty success. | `data.execution`, `error.code` | [VerifiedCompletion](../src/domain/identity.rs) |
| SESSION-007 | Canonical ID never becomes the caller's name. Unresolved identity is null in v2; caller reference remains separate. | `data.session_id`, `data.session_ref` | [busy_session_has_no_exit_and_no_stdout_alias](../tests/domain.rs) |
| SESSION-008 | PTY output is merged terminal content, never independent stdout/stderr. | `data.output.kind == pty`, `data.output.content` | [PTY DTO test](../tests/domain.rs:busy_session_has_no_exit_and_no_stdout_alias) |
| SESSION-009 | Recover sends interrupt under the writer lock and requires a fresh shell prompt. Never type an exit command into a REPL. Preserved means usable managed shell, not merely surviving tmux. | `data.session_preserved`, `data.foreground` | [recovery is explicit](../tests/live/session.rs:raw_send_exit_status_unknown_target_and_recovery_are_explicit) |
| SESSION-010 | A create that reached the host reserves its candidate identity before submitting and reports it with `creation_status` on failure: `not_created` (explicit refusal), `unknown` (timeout/cancellation/disconnect/missing evidence), or `created` (creation evidence without a full record). A local mistake or ungeneratable id reports `data: null`. A resolved `--cwd` is recorded as an absolute path and an old record falls back to its metadata literal. | `data.session_id`, `data.session_ref`, `data.creation_status`, `data.initial_cwd` | [Rust create tests](../src/session/ops/create.rs); [failure-shape schema cases](../examples/contract_cases.rs) |
| TUNNEL-001 | Alive means the forward exists, not application health. Close only the named dedicated master and its record. | `data.tunnel_id`, `data.tunnels[]` | [tunnel persistence](../tests/live/tunnel.rs:socks_tunnel_persists_across_cli_processes_and_closes_cleanly) |

## Files and environment

Source: [files](architecture/files-and-json.md), [runtime](architecture/runtime.md),
[engineering](architecture/engineering.md).

| ID | Required behavior | JSON path | Executable evidence |
| --- | --- | --- | --- |
| SSH-001 | System OpenSSH owns resolution, authentication and host keys. Never weaken checking or install remote packages. | `error.code`, `data.capabilities` | [OpenSSH options builder](../src/transport/openssh.rs) |
| SSH-002 | Shared-master state is observable; reset stops new channels without killing accepted channels. Fresh mode never initializes shared state. | `data.master_status`, `data.stopped` | [control-protocol only](../tests/acceptance/transport.rs:connection_status_and_reset_drive_the_control_protocol_only) |
| SSH-004 | `doctor` reports the binding each capability resolved to in this execution environment, under the same key set as `capabilities`, or null when absent. A failed probe reports both maps empty; rhost probes no sudo, group, full PATH, version or package ownership. | `data.capability_paths` | [capability-map unit tests](../src/doctor/app.rs); [probe acceptance tests](../tests/acceptance/transport.rs) |
| SSH-003 | Deep or space-containing cache paths must not break transport reuse. The hash/path algorithm is not frozen. | `data.control_path`, `data.master_status` | [deep cache fallback](../tests/live/transport.rs:deep_cache_path_falls_back_without_losing_reuse) |
| FS-001 | A remote path is one literal value; expand leading home shorthand remotely, reject paired outer quotes before any remote action. | `error.code`, `data.path` | [identities_paths_and_hashes_are_validated](../tests/domain.rs) |
| FS-002 | Plain put/get use scp; resume/checksum use rsync and verify digest. Parent creation is explicit. | `data.backend`, `data.checksum_verified` | [scp invocation test](../tests/acceptance/files.rs:a_put_builds_one_scp_invocation_and_reports_the_landing_file) |
| FS-003 | Read is bounded UTF-8 text with SHA-256 of the complete file. | `data.content`, `data.sha256`, `data.truncated` | [bounded read evidence](../tests/live/exec.rs:bounded_capture_reports_source_bytes) |
| FS-004 | `read`/`write`/`patch` name one exact regular file and refuse a symbolic link in any path component. Existing replacement and patch require a matching hash; replacement locks the already-held parent directory and uses a dirfd-relative temp plus rename or link, atomically and preserving permissions. | `error.code`, `data.sha256` | [FileWrite](../src/domain/files.rs); [helper symlink refusal](../tests/acceptance/remote_fs.rs:editing_refuses_a_symlinked_target_or_parent_without_following_it); [CLI symlink refusal](../tests/acceptance/edit.rs:the_editing_surface_refuses_symlinked_targets_without_reading_or_writing_through) |
| FS-005 | Sync/mirror never delete without explicit delete; a destructive remote destination is checked after realpath resolution to preserve dangerous-path refusal, and timeout process cleanup is preserved. | `data.delete`, `data.changes`, `error.code` | [sync delete is explicit](../tests/live/files.rs:sync_plan_apply_and_delete_are_explicit); [resolved root refusal](../tests/live/files.rs:transfer_failures_and_resolved_root_targets_are_refused) |
| FS-006 | Batch is ordered serial put/get, not a transaction. Continue after entry failures; completed report has ok=true, failed entries make process status 255. | `data.items[]`, `data.failed` | [batch report test](../tests/acceptance/edit.rs:a_batch_reports_every_entry_and_fails_the_process_only_for_failed_entries) |
| RELEASE-001 | Re-run the complete tagged-source gate on Linux and macOS, then execute the exact four shipping artifacts natively and verify their checksums before publication. All six jobs use the Rust version declared by Cargo. The local installer selects the same assets and verifies SHA-256 before replacement; plugin versions mirror Cargo, the sole binary version authority. | `version` result | [release workflow](../.github/workflows/release.yml); [release contract check](../scripts/check-release-contract.sh); [installer test](../scripts/test-install.sh) |

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
