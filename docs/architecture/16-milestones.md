[← Architecture map](../ARCHITECTURE.md)

# Part XVI — staged implementation

## 43. Milestone 0 — repository skeleton

Deliver:

```text
SKILL.md
README
docs
Go module
CLI command tree
JSON envelope
version command
```

No remote feature should be faked. Unimplemented commands should fail clearly.

---

## 44. Milestone 1 — host + doctor + exec

Implement:

```text
host resolution
OpenSSH process execution
ControlMaster namespace
doctor
exec
JSON/human output
```

Acceptance:

- a real client host → remote Linux/WSL2 call works;
- separate `rhost` processes reuse transport;
- stdout/stderr/exit code/timeout are correct.

Do not move on until this is solid.

---

## 45. Milestone 2 — sessions

Implement:

```text
tmux capability detection
create/list/close
managed metadata
output logging
session exec
session send/read
human attach
```

Acceptance:

- cwd/env persist across separate local CLI invocations;
- controller restart is irrelevant because no controller exists;
- session can be attached by a human;
- command boundary protocol is tested.

---

## 46. Milestone 3 — jobs

Implement:

```text
job start/list/status
remote metadata
stdout/stderr
offset logs
stop/kill
stale detection
```

Acceptance:

- a job survives CLI exit and SSH disconnect;
- remote process group can be stopped;
- completed jobs remain inspectable.

---

## 47. Milestone 4 — files

Implement:

```text
put/get
rsync-backed sync
dry-run
explicit delete
JSON result
```

Acceptance:

- source changes sync correctly;
- unrelated remote data is not deleted by default;
- destructive sync has a preview path.

Status: implemented and verified against a real remote Linux host over SSH
(`make test-live-fs`). Notes on how the requirements above were met:

- `put`/`get` use `scp`, `sync` uses `rsync`; both inherit rhost's own OpenSSH
  options, so `~/.ssh/config` stays the source of truth (§5, §27);
- `--delete` is never implied, and a `--delete` destination that is a top-level
  directory or a whole home is refused with `SYNC_REJECTED` before anything runs
  (§28); the rule is tested end to end, so the refusal provably precedes the copy;
- the dry-run plan is `data.changes[]` with a coarse `action`, the raw rsync
  itemize string, and `data.deletes` — one parsed shape for both GNU rsync and
  the openrsync shipped as a client on macOS, whose deletion lines ignore
  `--out-format`;
- `fs` is foreground and local-process-owned. It is deliberately outside the
  persistence invariant of §2: a transfer that must outlive the CLI belongs to a
  job or a resume protocol, not to this milestone.

---

## 48. Milestone 5 — status/watch

Implement:

```text
one-shot remote probe
Linux/WSL2 system model
optional NVIDIA probe
session/job aggregation
watch TUI
```

Acceptance:

- status works without NVIDIA;
- unsupported telemetry does not fail snapshot;
- watch survives temporary disconnect and reconstructs state.

Status: implemented and verified against a real remote Linux host over SSH
(`make test-live-status`). Notes on how the requirements above were met:

- `status` runs one bounded read-only probe (§30) over the normal exec path and
  parses a versioned `key=value` intermediate form, so an unsupported or
  unreadable metric is `null` and named in `data.unavailable`, never a zero and
  never a failed snapshot. A host with no `nvidia-smi` simply has no
  `accelerators`;
- `platform` records Linux vs WSL; the accelerator list is generic (today one
  row per `nvidia-smi` GPU);
- managed sessions and jobs are aggregated by reusing `session list` and
  `job list`, not by re-deriving remote state; a section that cannot be read is
  reported in `data.unavailable` while the rest of the snapshot stands;
- `watch` owns no state — it re-runs the same snapshot each interval and
  rediscovers sessions and jobs remotely, so an unreachable host is an
  `online: false` refresh with an `offline_code`, and the next success
  reconstructs everything from the host. It is also the one place `--json`
  streams rather than emitting once: one envelope per refresh, one per line;
- the snapshot's duration is reported as `data.probe_ms`, deliberately not as an
  RTT: a single probe cannot separate network round-trip time from remote
  execution time, and calling the sum an RTT would be a fabricated number.

---

## 49. Milestone 6 — hardening

Only after daily use:

```text
audit
output truncation policy
config migration
shell recovery
more robust process cancellation
installer
release signing/checksums
```

Then evaluate whether a local daemon is actually justified.

---

