# Migration friction log

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
