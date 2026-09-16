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

None.

An entry appears here only when one of the three triggers above actually
happens. Once resolved, its durable decision goes to the contract or maintenance
policy, its lineage to git (or `tests/fixtures/history.json` if it was a real
defect), and the entry is deleted.
