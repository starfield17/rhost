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
