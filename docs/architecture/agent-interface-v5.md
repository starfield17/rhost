# Agent interface evolution — architecture review proposal

Status: proposed, not implemented. The active product remains v4 with schema v2.
This document is the review packet required by MAINTENANCE.md for Risk A work;
it does not change an existing acceptance criterion or claim new capabilities.

## Final v5 decisions (supersede staged migration below)

All v5 public JSON operations will emit schema v3; v2 remains frozen history.
There is no workspace, worktree, branch, project snapshot or backup management.
Agents own those workflows. File recovery records cover only explicit edits.
Use LLVM/Clang and LLD with pinned musl build inputs for local helper builds;
CI verifies uploaded helper binaries and never compiles replacements. Preserve
four native client artifacts. Publish only after the complete implementation,
same-commit verification and exact-artifact live acceptance pass.
Read-only probes do not create namespaces or return invented generations.

## Consumers and compatibility

Affected consumers are shell callers, JSON validators, coding agents, existing
tmux session users, installers, release automation and operators of remote hosts.
Existing `exec`, `session`, transfer and connection semantics remain unchanged in
the first delivery. New request/Run operations require schema v3; never label a
new closed-object operation as v2. Existing v2 compatibility fixtures are frozen.
No automatic adoption or deletion of v4 remote state is permitted.

The first delivery is a vertical slice: a C helper, recoverable noninteractive
Runs, structured requests, bounded incremental output, and verification of the
exact helper binary before publication. File change sets and MCP follow only
after this slice passes its acceptance scenarios.

## Ownership and implementation boundaries

- OpenSSH retains authentication, target resolution, host keys and multiplexing.
- A remote monitor exists per Run and exits after recording its result. There
  is no global daemon, listening network service, scheduler or automatic restart.
- Durable requests, logs and results belong to remote files, never CLI memory.
- An independent C99 executable owns process and filesystem primitives. Rust
  owns public JSON, CLI parsing, orchestration and error presentation. No FFI.
- POSIX APIs form the baseline. Linux-specific process identities and syscalls
  are optional implementations of a guarantee, not the public interface.
- The C implementation is a narrow exception to AGENTS.md's Rust-only rule;
  new acceptance tests remain Rust. C sources require bounds checks, a source
  size gate and local address/undefined-behavior sanitizer verification.

Implementation owners: `src/runs/` owns CLI, DTOs and Run use cases;
`src/helper/` owns helper selection, preparation and the private protocol;
`helper/` owns the standalone C program. `domain/` owns pure Run transitions;
`wire/` retains envelope ownership. `dispatch/` composes capabilities. Existing
capabilities do not call the Run CLI or depend on its rendering.

## Request and execution semantics

`rhost request --from -` reads one bounded JSON request. It is a single-operation
entry point, not a workflow engine. `run start`, `poll`, `list`, `signal` and
`prune` are CLI views of the same typed requests. Reject invalid requests before
contacting SSH. Program input is an explicit request field, not unread bytes
following the JSON document. The first slice accepts closed stdin or bounded
base64 input; interactive input stays with sessions.

Run start requires a caller-generated request key, an expiry, cwd, explicit
environment policy and exactly one program variant: an argv array or an exact
shell program with an explicit interpreter. No shell syntax is reconstructed
from argv. Shell profile loading is explicit. The request fingerprint covers
program, input, resolved cwd, environment policy and deadline.

Default waiting budget is zero; a positive `wait` only waits for an observation.
A separate optional deadline is enforced remotely. Local cancellation of a wait
does not cancel a Run. Signal is an explicit operation. Current foreground exec
timeout/cancellation behavior remains unchanged.

Remote state is private to the effective remote account under a new namespace.
Registration uses an exclusive request lock and a durable atomic record before
spawning. Concurrent identical submissions resolve to one Run; a different
fingerprint under the same key returns `IDEMPOTENCY_CONFLICT`. A registered
request is never automatically respawned after ambiguous launch failure.

Expiry is part of the key contract: an expired request is refused even after its
record has been pruned. Completed results may be pruned after expiry; active Runs
are never pruned. Source clocks and expiry bounds are checked on the remote host.
This is bounded submission deduplication, not exactly-once business effects.

| State | Evidence | Allowed next state |
| --- | --- | --- |
| accepted | durable request, no child evidence | starting, lost |
| starting | monitor owns launch attempt | running, exited, lost |
| running | monitor and child identity validated | exited, lost |
| exited | durable wait result bound to this Run | terminal |
| lost | launch/completion cannot be established | exited only if authentic completion is recovered |

Transport availability is separate from Run state. A failed poll preserves the
last observation as stale and returns a transport error; it cannot assert that
the Run is currently running or lost. A missing result is never exit code zero.
Exit code and signal termination are distinct results.

The monitor owns child process identity and receives control over a private
local socket. It validates its Run identity before accepting signals. A missing
monitor is not permission to signal a saved numeric PID. Process-group cleanup
does not cover children that escape that group; full containment is not promised.
Host reboot or account-session cleanup may kill the monitor and yields lost
evidence, not automatic re-execution. A successful capability probe cannot
guarantee survival under every future host policy.

## Output and storage

Each Run has separate stdout/stderr append-only segments and an atomically
written completion record. Cursors are absolute source-byte offsets bound to
Run and stream; repeated reads of retained ranges return identical bytes.
JSON text decoding never changes cursor accounting. Binary pages use base64.

The first implementation uses a 64 MiB quota per stream and a 4 KiB default
inline page (maximum 1 MiB). Once a stream's quota is reached, the monitor keeps
draining and counting bytes but discards subsequent bytes, recording the missing
range explicitly. It never blocks a child indefinitely to retain its output.
Completion includes final byte counts and output completeness. Disk write
failure is recorded when possible and never fabricated as complete output.

Default request validity and completed-result retention are 24 hours; callers
may request up to seven days. An expired key is never interpreted as new work.
Pruning is explicit in the first delivery; no hidden background GC. Host-wide
storage exhaustion refuses new Runs before launching whenever detectable.

## Helper preparation

Selection order: validated cache; configured local cross compiler; existing
remote `cc`; a matching verified Release artifact. Never install a compiler,
package or SDK automatically. A configured no-upload policy refuses preparation
when a compatible cached helper is absent.

Cache identity includes source digest, target OS/architecture/ABI, protocol
version and build configuration. A content hash alone does not establish source
provenance. Source compilation uses fixed argv and isolated output paths, no
user-provided build script. Temporary preparation is atomic under a lock; an
interrupted build cannot replace a working helper. Keep helpers used by active
monitors. Missing compiler, noexec filesystem, incompatible ABI and protocol
mismatch have distinct machine-readable refusals.

Bootstrap runs through minimal SSH command execution; it cannot depend on the
existing bash/setsid wrapper or on Python. A readiness handshake reports actual
operation guarantees and protocol limits. No universal Unix support claim is
made solely because source compilation succeeds.

## Artifact verification and release

Local builds produce Linux x86_64/aarch64 static musl candidates first. The
candidate manifest includes source commit and source digest, target triple,
protocol version, compiler/configuration and binary SHA-256. Build metadata is
a claim; the test result identifies which exact bytes were exercised.

Use a candidate release name outside the production `v*` trigger. A dedicated
helper verify workflow supports manual dispatch and reusable workflow calls.
It downloads candidates from this repository, validates manifest/digest and
checks compatibility against the checked-out source. It never compiles the
helper. Building Rust test harnesses and clients remains permitted.

Verification runs on native Linux x86_64 and aarch64 runners, both directly and
through a temporary sshd with runtime-generated credentials and known_hosts.
Do not disable host-key checks. A test target is supplied through the environment
and absent targets fail with `set RHOST_TEST_HOST`. No target, credentials or
real-host transcript belongs in tracked content.

The workflow stores the tested binary unchanged plus evidence naming the test
commit, binary digest, platform and workflow run. Publication consumes that
verified artifact, not a new build or a second download from a mutable URL.
The production release workflow calls verification at the tagged source commit
before publication. Existing native main-program verification remains required.
Linux CI evidence is not macOS/BSD remote evidence or old-kernel evidence.

## Review resolutions required before implementation

The independent architecture review found the initial proposal incomplete.
The following decisions supersede less specific descriptions above:

### Identity, expiry and launch

Request keys have the form `<expiry-unix-seconds>-<32-lowercase-hex-random-bytes>`.
Expiry is immutable within a key, matches the request field and participates in
the fingerprint. Changing expiry is a new submission, never a retry. Effective
remote time is the maximum of wall time and a namespace-wide durable high-water
mark. Update that mark under an exclusive lock and fsync before registration or
pruning; refuse new submissions if persistence fails. Clock jumps forward can
expire requests early and return an explicit expiry refusal.

A namespace has a durable random generation required by mutating requests.
Replacing or restoring state invalidates deduplication promises; old requests
must not be replayed into a replacement namespace. This boundary is explicit,
not an unconditional exactly-once claim.

Registration uses a version-1 closed record: namespace generation, key, expiry,
fingerprint, Run id and accepted state. Write request and metadata in a private
staging directory; fsync files and directory; rename under the registry lock;
fsync the registry parent before launching. The monitor reads the committed
request independently of SSH stdin. The submitter holds a per-request launch
lock, inherited by the monitor, until readiness is durably published. A missing
readiness record is starting, not permission to respawn. Reconciliation marks
lost only after acquiring the same lock and proving that no matching monitor
owns the request. Unknown record versions are refused, never treated as absent.

### Signals and stream lifetime

Keep the owned process-group leader unreaped while group signaling is enabled.
Observe exit without reaping, permanently disable control, then reap and publish
the wait result. Platforms without non-reaping child observation cannot advertise
this signaling guarantee. After leader exit, surviving group members are outside
Run control; no saved PGID is signaled afterward.

Publish process completion independently of pipe EOF. Descendants may retain
stdout/stderr pipes: drain for at most two seconds after leader exit, then close
remaining readers, record an unobserved-tail indicator and exit the monitor.
Poll reports stream openness separately from process exit. Counts cover bytes
actually observed; neither EOF nor leader exit proves whole-tree completion.

### Protocol and preparation

Private protocol v1 has a 16-byte header: ASCII `RH01`, big-endian uint32 message
kind, big-endian uint64 body length. Limit requests to 8 MiB. Bodies use typed,
fixed-order length-prefixed UTF-8/raw fields; public JSON stays in Rust. Accept
one frame followed by EOF; reject unknown kinds, overflow, missing fields, NUL
in argv/path/env and trailing bytes before mutation. Responses use the same
framing. Child streams never carry control messages. A read-only handshake
returns protocol/source/target identity and supported operation guarantees.

Create mode-0700 staging beneath an account-owned non-symlink state root.
Traverse using no-follow directory-relative opens; verify ownership and mode
before reuse. Write and fsync a candidate, run its handshake in staging, then
rename atomically and fsync the cache directory under the preparation lock.
Never overwrite an executable in place. Refuse unsupported filesystem locking
or durability semantics rather than assuming a network filesystem is equivalent.

### Schema and artifact promotion

Preserved operations continue emitting v2; only request/Run operations, including
their help/error refusals, emit v3. A v3 metadata request advertises supported
generations without modifying the closed v2 version result. Closed v3 request
and result schemas and valid/invalid fixtures precede command wiring. A uniform
v3 migration is a later explicit compatibility decision.

Helper verification dispatch inputs are a full source commit, candidate asset
IDs and expected manifest SHA-256. The manifest includes both architecture
digests. Release consumes artifact IDs from a successful verification run bound
to the tagged test commit and manifest digest, and rechecks both binary hashes.
Absent, expired or replaced artifacts refuse publication. The builder is trusted
to supply the claimed source; this is behavioral verification, not independent
cryptographic source provenance or reproducible-build proof.

Existing check requirements remain enforced. Additive C/schema/release checks
need separate measurement-only changes; "unchanged" below does not prohibit
those reviewed additions.

## Subsequent slices

File errors must distinguish not-applied, applied, partial and unknown mutation
evidence. A post-write verification conflict cannot imply no write occurred.
Multi-file changes preflight all hashes and journal individual applications;
they do not promise atomic visibility to arbitrary external readers. Rollback
checks the installed version before restoring and never overwrites a later edit.
Validation with side effects is not undone by restoring file bytes. Worktree or
copy isolation is a separate operation; dirty/untracked user work is preserved.

Structured search/read returns version-bound content and continuation cursors.
An advisory lock coordinates participating writers only. Environment identity
reports observations and guards; it does not claim hermetic execution.

MCP maps the same typed requests and outcomes without shelling out to CLI argv.
Transport cancellation ends observation; Run cancellation remains explicit.
General-purpose batch, dependency graphs and scheduling are deferred.

## Acceptance and review checklist

No implementation milestone is complete until these scenarios pass:

1. Independent CLI processes rediscover a Run after client exit and SSH loss.
2. Same-key concurrent submissions start at most one child; changed requests
   conflict; expired/pruned keys cannot silently start another child.
3. Fault injection at registration, spawn and result publication preserves
   uncertainty. Monitor death never authorizes a duplicate launch or PID kill.
4. Known exit and signal termination remain distinct; wait expiry is not stop.
5. Stream separation, byte cursors, repeated pages, invalid UTF-8, quotas and
   disk failures preserve counts and explicit output gaps.
6. Preparation without Python/tmux/cc uses the exact prebuilt candidate;
   interrupted preparation, noexec and mismatched protocols refuse honestly.
7. Native Linux and real SSH paths exercise the shipping helper bytes; changed
   candidate hashes invalidate evidence and prevent publication.
8. Existing `make test-smoke` and `make check` pass unchanged. Native live suites
   remain explicit manual verification until separately bound to release.

Architecture review must resolve request expiry/clock behavior, monitor signal
identity, disk-failure durability, bootstrap trust, schema migration and artifact
promotion before implementation. Review is not satisfied by this author's own
description. Acceptance/gate changes land separately from implementation under
the existing GateChange policy. The active contract is not relaxed to fit a new
implementation.
