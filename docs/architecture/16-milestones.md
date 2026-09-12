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
- managed sessions and jobs are aggregated by running the same list scripts
  `session list` and `job list` use, inside the one snapshot SSH, and parsing
  them with the same functions; a section that cannot be read is reported in
  `data.unavailable` while the rest of the snapshot stands;
- `watch` owns no state — it re-runs the same snapshot each interval and
  rediscovers sessions and jobs remotely, so an unreachable host is an
  `online: false` refresh with an `offline_code`, and the next success
  reconstructs everything from the host. It is also the one place `--json`
  streams rather than emitting once: one envelope per refresh, one per line;
- the snapshot's duration is reported as `data.probe_ms`, deliberately not as an
  RTT: a single probe cannot separate network round-trip time from remote
  execution time, and calling the sum an RTT would be a fabricated number.

---

## 49. Milestone 6 — hardening and remote development tools

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

Status: implemented, except for the three items named at the end of this section.
Verified against a real remote Linux host over SSH — `make test-live-tools` for
the file, exec and tunnel surface, and `make test-live-session` for recovery
(both part of `make test-live-all`):

- **audit**: implemented (§36) — a local JSONL trail of remote operations,
  fail-open, disabled with `RHOST_AUDIT=0`, read back with `rhost audit`.
- **more robust process cancellation**: a timed-out `fs` transfer now runs in its
  own process group and kills the whole group on cancellation, with `WaitDelay`
  as a backstop, so the transfer is stopped rather than abandoned when a child
  holds the stdout/stderr pipes.
- **output truncation policy**: one rule for every command that reads the remote
  side — a limit is always *reported*, never silently applied. `exec` and
  `exec-many` take `--max-output-bytes` (1 MiB default, `0` for unlimited) and
  answer with `stdout_truncated`/`stderr_truncated` plus the remote byte counts;
  the capture keeps the beginning *and* the end of a stream, because the tail of a
  failed command is where its reason is; a bounded output still proves the command
  finished, since the completion marker is looked for in the whole capture and is
  never evicted by the bound.
- **shell recovery**: `session exec` reports `session_preserved`, and
  `rhost session recover` is the explicit repair path — interrupt, then prove the
  pane answers again. It never recreates a session behind the caller's back
  (§20), which is why it reports a lost session instead of making a new one.

The same round added the tools that daily use asked for, and they are listed here
rather than in a new milestone because they share one purpose: letting an agent do
source work on the remote without downloading the tree.

- **remote file operations** (`fs read/write/patch/grep/glob`, Part VIII): one
  embedded, dependency-free Python helper answers a JSON request over the existing
  exec transport, so no path or file content is ever re-parsed by a shell (§28's
  lesson applied to the read side). Writes are compare-and-swap on the hash that
  `read` returned, atomic within the directory, and refuse symlink targets; the
  helper's own error codes are part of §33.
- **directory download and ordered transfer** (`fs mirror`, `fs batch`): `mirror`
  is `sync` with the direction stated in the command, and `batch` is one manifest
  of `put`/`get` pairs whose envelope keeps per-entry results — a partial failure
  is a report, not a truncation of the run.
- **verified transfer** (`--resume`, `--checksum`): rsync's own partial-file and
  integrity mechanisms for a single file, plus an end-to-end SHA-256 comparison
  that does not depend on the transfer tool's opinion of what it copied.
  `resume_enabled` describes what was *asked for*; `checksum_verified` describes
  what was *proven*.
- **bounded parallel execution** (`exec-many`): several targets, one command, a
  worker cap, and per-target rows; it is foreground orchestration and does not
  pretend to be a durable scheduler (§7's job model stays the only one).
- **tunnels** (Part IV): three forwarding kinds on a dedicated OpenSSH master per
  forward, with a record outside the CLI process so the forward outlives the
  invocation that made it (§2), loopback-only by default, and `list` reporting
  OpenSSH's own answer about each master rather than a guess from the record.

The hardening round closed two lifecycle holes that could reach a real remote
machine, and both are now contract rather than caveat:

- **process identity** (§22, §25): a job's pid is not its identity. Each job
  records its boot id and process start time in `identity` before its pid file;
  a job is `running` only while those still describe the live pid, and
  `job stop`/`job kill` signal only a verified process group. A pid that is now
  another process is `stale` and is never signalled — `signalled: false` in the
  JSON says a refusal happened, instead of a stop that did not.
- **session foreground safety** (§16): `session exec` refuses with
  `SESSION_BUSY` when a program owns the pane instead of pasting into it, and the
  attach path toggles echo on the pane's pty instead of sending `stty` as input.

Still open: **config migration**, **release signing**, and the daemon evaluation.
Release artifacts carry a build-provenance attestation — the workflow, repository
and commit behind the bytes, verifiable with `gh attestation verify` — which is
provenance and not a signature. The installer's checksum path is implemented *and*
exercised end to end: `scripts/install.sh` downloads a release asset plus its
`.sha256`, verifies it, and replaces the binary atomically, and installs the
newest release of any kind while every release is a prerelease. The release
pipeline that produces those artifacts is §38.

---
