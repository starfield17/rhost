[← Architecture map](../ARCHITECTURE.md)

# Part XVI — implementation history

## 43. Milestone 0 — repository skeleton

Established the Go binary, JSON envelope, release pipeline, contributor rules,
and portability gate.

## 44. Milestone 1 — host, doctor, and foreground execution

Established OpenSSH as the configuration and authentication authority,
ControlMaster reuse, command completion markers, timeouts, and real exit codes.
The primary interface now accepts one shell string in a remote context, streams
ordinary I/O, and handles cancellation without owning durable state.

## 45. Milestone 2 — sessions

Added remote tmux sessions with serialized writers, explicit command boundaries,
incremental reads, attach, and recovery.

## 46. Milestone 3 — jobs

Added detached process groups, remote metadata and logs, verified process
identity, incremental reads, and conservative signal handling.

## 47. Milestone 4 — files

Added foreground transfer and sync, explicit deletion preview, verified/resumable
single-file transfer, directory mirror, ordered batches, and hash-protected
atomic text writes.

## 49. Milestone 6 — hardening and surface reduction

Added bounded structured output, audit, installer and release checks, tunnel
lifetime, session recovery, remote process cleanup, and direct streaming stdin /
stdout / stderr.

Daily use then narrowed the product: host monitoring, multi-host fan-out, and
remote search wrappers were removed. Their behavior is composed with ordinary
remote commands and caller-side shell control. Specialized operations remain
only where they provide persistence, transfer, or concurrency-safety guarantees
that a one-line remote command cannot cheaply supply.

Still open: config migration, release signing, and evaluating a daemon only if
measured process-start or platform constraints justify one.
