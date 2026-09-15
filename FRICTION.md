# Engineering friction log

Append observations and resolutions; do not rewrite earlier entries to make a
later implementation look consistent. A passing local test is not live evidence.

## 2026-09-14 — public CLI validation differs from application validation

- Observation: empty `--command` returns `USAGE_ERROR`; whitespace-only input reaches application validation and returns `CONFIG_INVALID`.
- Evidence: `internal/cli/command_input.go`, `internal/app/exec_stream.go`, and the binary conformance validation cases.
- Impact: a naive acceptance test expecting `CONFIG_INVALID` for both would invent a compatibility change.
- Decision: retain both existing public results in WIRE-006; correct the new test specification, leave Go behavior unchanged.
- Follow-up: Rust CLI wiring must preserve the distinction.

## 2026-09-14 — missing completion after a zero SSH status

- Observation: Go reports `INTERNAL` when SSH exits zero but no completion marker is present; architecture emphasizes execution uncertainty.
- Evidence: `internal/app/classify.go` and `TestClassifyMissingMarker`.
- Impact: treating Go as an absolute oracle would freeze a mechanism-specific classification.
- Decision: v1 keeps its tested behavior; v2 uses `REMOTE_EXECUTION_UNKNOWN`, never success or retryable connectivity failure.
- Follow-up: transport implementation needs a black-box regression for this explicit version difference.

## 2026-09-14 — historical subprocess tests depend on Go marker syntax

- Observation: two exec regressions synthesize private Go completion markers; extracting them literally would freeze the old protocol.
- Evidence: `TestCompletedForegroundSurvivesInheritedBackgroundPipe` and `TestCancellationAfterCompletionKeepsForegroundExitCode`.
- Impact: Rust DTO/domain coverage is not independent real-process evidence for these races.
- Decision: retain the original Go tests, list HIST-PROC-002/003 gaps, and gate transport parity on protocol-independent live scenarios.
- Follow-up: drive the real remote wrapper and synchronize through external observations before cancellation; do not synthesize candidate markers or add timing sleeps as a race fix.

## 2026-09-14 — UTF-8 rendering is not byte accounting

- Observation: replacing one invalid source byte with a Unicode replacement character expands JSON text length.
- Evidence: `invalid_utf8_does_not_corrupt_source_byte_evidence`.
- Impact: a text-only constructor checking rendered length against source bytes rejects legitimate Unix process output.
- Decision: store the captured raw bytes, derive truncation from source bytes, and perform lossy UTF-8 rendering only in the DTO projection.
- Follow-up: transport must preserve raw human streams and count observed bytes independently of capture and delivery.

## 2026-09-14 — zero-status missing-evidence regression extracted

- Observation: this case can be exercised without knowing marker syntax by a stub OpenSSH process that emits nothing and exits zero.
- Evidence: `TestConformanceMissingCompletionEvenWithZeroSSHStatus`.
- Impact: the deliberate v1/v2 classification difference is now executable against either binary.
- Decision: assert `INTERNAL` only for v1, `REMOTE_EXECUTION_UNKNOWN` for v2, and non-retryability for both.
- Follow-up: Rust transport must pass this existing acceptance case.

## Legacy freeze replaces active Go development
- Trigger: owner requested all Go code and version authorities be archived.
- Constraint: retain executable historical evidence without maintaining two implementations.
- Decision: freeze the original tracked tree plus the extracted standalone harness; active checks and future tests use Rust.
- Evidence: archive SHA-256 test and base-commit CI guard; explicit optional reference commands.
- Follow-up: port remaining independent acceptance scenarios to Rust without weakening their semantics.

## 2026-09-14 — EXEC-008 races get live Rust evidence without a timed signal

- Observation: the two exec regressions that had no independent scenario either
  fabricated Go completion markers (unusable, since the marker mechanism is not
  frozen) or needed an interruption timed against the drain window.
- Evidence: `tests/live_exec.rs`, run by `make test-live-rust` against a real
  host; the inherited-pipe case asserts a completed run, and the interruption
  case is ended by the `--timeout` its caller declared, so the completion is real
  and the interruption is scheduled by the contract rather than by a sleep.
- Impact: a SIGTERM version of the second case was implemented first and removed:
  its synchronization was a wall-clock sleep, which the earlier entry on this
  page forbids, and a black-box CLI exposes no event between "completion
  observed" and "drain finished" to synchronize on instead.
- Decision: cover EXEC-008 live with the declared deadline, keep the hermetic
  cancellation case and the frozen `TestLiveExecDirectCancellation` for the
  signal path, and leave the marker protocol unparsed by every candidate test.
- Follow-up: if a future envelope or console event marks "completion observed"
  before the run ends, the signal variant can be added with a real observation.

## 2026-09-14 — the audit trail belongs to v4's namespace

- Observation: the ledger (PERSIST-002) and the migration policy both place the
  local audit trail at `RHOST_STATE_DIR/v4/`, but the implementation appended to
  the shared `RHOST_STATE_DIR/audit.jsonl` that the v1 binary also used, and the
  acceptance tests encoded that path.
- Evidence: CONTRACT.md PERSIST-002; docs/RUST_MIGRATION.md "State and upgrade
  policy"; the three `audit::path`/`Recorder::from_env` call sites;
  `tests/acceptance/audit.rs`.
- Impact: v4 would have read and appended to a v3 installation's trail, which is
  the adoption PERSIST-002 forbids, and the documented path would have been a
  claim the binary did not honour.
- Decision: the ledger wins. The trail moved to `RHOST_STATE_DIR/v4/audit.jsonl`,
  the acceptance paths followed, and the policy's "the audit trail later" now
  reads as what it is. No compatibility reader was added.
- Follow-up: an upgrading user's v3 trail stays on disk and stays readable with
  v3; `rhost audit` reports this version's own operations.

## 2026-09-14 — shell capability probes do not share a missing-command status

- Observation: `sh -c 'command -v missing'` commonly exits 1, while the Rust
  rsync probe treated only 127 as a missing dependency.
- Impact: a host without rsync was reported as `TRANSFER_FAILED` instead of the
  stable, non-retryable `REMOTE_DEPENDENCY_MISSING` capability answer.
- Decision: transport uncertainty remains classified before the verdict; every
  completed nonzero answer from this yes/no probe means rsync is absent.
- Follow-up: retain direct verdict coverage for both 1 and 127.

## 2026-09-14 — remote state overrides name the complete v4 root

- Observation: doctor, sessions and cleanup honored `RHOST_REMOTE_STATE`, but the
  foreground exec wrapper wrote pid evidence to the default directory.
- Impact: cleanup could look in a different directory from the one exec used and
  report an avoidable unknown result.
- Decision: every remote component resolves
  `${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost/v4}`; an override is already the
  complete root and receives no additional version suffix.
- Follow-up: keep wrapper and cleanup expressions in one protocol regression.

## 2026-09-14 — shell completion is not an agent operation

- Observation: Go exposed Cobra's generated `completion` convenience command,
  but the closed v2 operation set does not include shell completion.
- Impact: reintroducing it would add four generated shell surfaces or expand the
  wire contract for little agent-facing value.
- Decision: do not migrate it; document the omission as intentional rather than
  treating it as an incomplete v4 operation.

## 2026-09-14 — a distribution manifest is not a binary version authority

- Observation: the bootstrap archive test rejected any root `plugin.json` as a
  restored Go entry point, while the product distribution again needs the
  portable Agent Plugin manifest alongside the current Codex manifest.
- Impact: retaining the broad absence assertion would forbid the requested agent
  package even when builds and artifact names read only `Cargo.toml`.
- Decision: continue forbidding the Go module, root VERSION, source entry points,
  and any workflow version input other than Cargo. Permit both plugin manifests
  as checked distribution mirrors whose versions must equal Cargo's package
  version.
- Follow-up: the Rust archive test and agent/release checks all fail on manifest
  drift; neither manifest is read by a build.

## 2026-09-15 — create uncertainty is reported, not erased

- Observation: a `session create` whose response was lost left the caller with
  `data: null`, even though the remote may have created the tmux session and
  written its `meta.json` before the channel died.
- Impact: an agent could not tell "definitely nothing" from "maybe a session
  exists", and a blind retry could leak an orphan session.
- Decision: reserve the candidate identity before submitting and report it with
  `creation_status` (`not_created`/`unknown`/`created`) on every failure that
  reached the host (SESSION-010). A failure that never reached the host keeps
  `data: null`. The transport cause and retryability are unchanged.
- Follow-up: `create` now reads the partial transcript. The remote prints its
  created-evidence line only *after* the record is on disk and the cleanup trap
  is disarmed, so evidence a killed helper leaves behind is honest: the session
  it names is not one the trap then removes. A live full-suite run showed the
  earlier "print early" ordering could name a session the trap had already
  deleted.

## 2026-09-15 — initial_cwd records the directory, not the request

- Observation: `session create --cwd '~'` stored the literal `~` in the record,
  so `create` and a later `list` reported a value that is not a directory and
  would not match the session's own `pwd`.
- Impact: a caller comparing the recorded cwd to live state saw a spurious
  mismatch; `~` is a shell shorthand, not a path.
- Decision: resolve `--cwd` in a remote subshell, record the absolute `$PWD` as
  a `cwd` sidecar beside `meta.json`, and report it from both `create` and
  `list`. An old record without a sidecar falls back to its metadata literal —
  no migration, no rewrite.
- Follow-up: the sidecar is raw bytes, not JSON, so a path with any characters
  round-trips without escaping.

## 2026-09-15 — a missing capability is unit-testable, not portably black-box

- Observation: `doctor` must report `null` for a capability the host lacks, but
  a hermetic black-box test cannot force one to be missing: the acceptance
  harness runs the probe under the local login shell, whose profile rebuilds
  `PATH` (macOS `path_helper`, `/etc/profile`) so `/usr/bin` tools always resolve.
- Impact: a black-box "missing" case would either be flaky across hosts or would
  have to parse the wrapper's private base64 protocol, which the harness
  deliberately does not do.
- Decision: cover the missing semantics deterministically in the
  `capability_maps` unit test (present → path, missing → null, empty probe → all
  null) and cover "a probe that never completed reports both maps empty" in the
  black-box suite. The success case asserts the two maps always share one key set.
- Follow-up: the live `doctor_capability_paths_match_a_real_command_v` case
  proves presence against a real `command -v`; absence needs no real host.

## 2026-09-15 — a transient transfer-deadline acceptance flake

- Observation: `files::a_transfer_deadline_stops_the_whole_tool_group` failed
  once during a full `make test-smoke` run, then passed on its own and in the
  immediate full re-run, with no code change between.
- Impact: it is unrelated to the session/doctor work in this change and must not
  be folded into it as if the fix had addressed it.
- Decision: record it here as a separate observation; do not weaken or widen the
  test to hide it. If it recurs, investigate the transfer deadline timing on its
  own.
- Follow-up: none in this change; the test is left exactly as it was.
