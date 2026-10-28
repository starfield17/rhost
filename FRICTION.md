# Engineering friction queue

This is the reverse channel for work **in flight**: the place an implementation
notes that a stated rule was wrong or ambiguous, that the design forced bad code,
or that the first correct-looking approach was blocked by something no document
mentioned. It is not a development diary. An entry leaves this file the moment
it is resolved.

Rules:

- Unresolved only, newest first. A resolved entry is deleted, not struck through.
- A durable semantic decision moves to `docs/CONTRACT.md` or
  `docs/MAINTENANCE.md`; a behavior that a test already fixes needs no prose.
  Everything else is already in git history — do not archive it here.
- Keep it to the four fields below. This file is read by an agent on every
  session, so length is a cost paid by every later task.
- A misfire of `scripts/check-integrity.sh` belongs here; the measurement is not
  weakened to silence it.

Format:

```text
## YYYY-MM-DD — <one-line constraint under pressure>

- Constraint: the rule or design decision that did not fit.
- What happened: the concrete thing that blocked or distorted the work.
- Current workaround: what the code does today instead.
- Proposed resolution: what would remove the friction, or what evidence would.
```

## Open

## 2026-10-28 — candidate installation failed once in parallel CLI acceptance

- Constraint: real candidate failures must be explained without weakening
  acceptance or treating a passing rerun as a diagnosis.
- What happened: one public Run CLI fixture's initial helper installation
  returned status 255 without diagnostics while another parallel case passed;
  the same case passed alone. The failed fixture directory was no longer present.
- Current workaround: none. Fixture names now include an atomic sequence rather
  than relying on clock uniqueness; production deadlines and assertions remain.
- Proposed resolution: retain installer-stage diagnostics and establish whether
  shared fixture names or a candidate handshake failure caused the observation.

## 2026-09-16 — session live test timed out once under a full serial run

- Constraint: a live test that fails once must be recorded without weakening it
  (docs/MAINTENANCE.md#regression-memory-and-flaky-tests), and live suites are
  required manual pre-release verification.
- What happened: on the v4.4.1 pre-release `make test-live-all`,
  `session::raw_send_exit_status_unknown_target_and_recovery_are_explicit` timed
  out on `sh -c 'exit 4'` with `REMOTE_COMMAND_TIMEOUT`, on real remote host
  over SSH. The three v4.4.1 commits touch no session or transport code. The
  same test passed in isolation, and a full re-run of `test-live-all` passed
  23/23.
- Current workaround: none; the test was left unchanged (no widened timeout, no
  retry), and the release proceeded on the clean re-run.
- Proposed resolution: run the isolated session case repeatedly on a real
  target, recording elapsed time and process evidence. Compare a cold pane with
  a warm pane; a per-command budget that assumes a warm pane is the leading
  suspect. Keep the entry until the observation is explained or fixed.


An entry appears here only when one of the three triggers above actually
happens. Once resolved, its durable decision goes to the contract or maintenance
policy, its lineage to git (or `tests/fixtures/history.json` if it was a real
defect), and the entry is deleted.


### Native file dependency acceptance

The v4 acceptance case required file editing to fail when remote Python was
unusable. That conflicts with the approved standalone C helper boundary. The
replacement acceptance requires the same edit to succeed with unusable Python,
keeps the existing CAS, permissions, path and bounded-read assertions, and runs
the direct conformance suite against an explicitly supplied native candidate.
No missing-candidate skip or Python production fallback is permitted.


### Uniform v3 acceptance

The approved v5 compatibility decision moves every public envelope to v3.
Existing v2 fixtures and the published v4.1.0 validator remain untouched. Active
DTO and CLI tests now require v3; the preserved payload projection additionally
checks the frozen v2 validator. File write/patch success reports applied mutation;
errors distinguish not_applied, applied, partial and unknown evidence.


### MCP cancelled-response oracle

The new queued-cancellation test incorrectly required a tools/call reply after
sending a cancellation notification. It reached the response deadline because
rmcp deliberately removes the cancelled request from its response pool and drops
that reply. This was an invalid delivery expectation, not evidence of a stuck
queue. The stdio acceptance instead verifies that cancelled queued mutations stay
unsubmitted when capacity returns; a direct admission-state test requires waiting
to end without obtaining a permit. EOF-during-active-observation acceptance stays
in place and requires prompt shutdown without signaling the Run.


### Concurrent MCP submission oracle

The per-operation occupancy measurement initially reused the harness's shared
line log. Concurrent SSH stand-ins can interleave their argv lines, so counting
records containing a poll marker undercounted real submissions and hit the
fixture deadline. MCP occupancy now uses one private marker file per submitted
poll process; it still requires all 16 active poll submissions before queuing a
mutation. The shared diagnostic log is not a concurrency synchronization oracle.
