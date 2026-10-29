# Native agent interface (v5)

Status: implemented, with process-level acceptance and real Linux SSH
verification. The active public envelope is schema v3 for every operation.
Frozen v2 payload compatibility remains covered independently.

## Boundaries and owners

OpenSSH owns authentication, host keys, SSH configuration and connection reuse.
The client invokes a standalone C99 helper; there is no Rust FFI, global remote
daemon, network listener, scheduler or automatic restart. A monitor exists only
for its Run and exits after recording completion and bounded output draining.
Remote files own registration, output and results across client exits.

`src/helper/` owns preparation and private transport; `helper/` owns native
process/filesystem primitives; `src/runs/` owns Run requests and observations.
`src/files/` owns edits, inspection and named changeset recovery. `src/request/`
composes closed requests; `src/mcp/` adapts them through the official stdio SDK.
The pure domain and versioned wire envelope remain separate owners.

Agents manage project directories, Git branches and worktrees themselves. rhost
creates no project workspace, snapshot or backup service. Changeset archives
cover only explicitly named files. General batch/DAG/workflow execution is out
of scope; the existing serial transfer batch retains its published semantics.

## Submission, identity and time

`request --from -` accepts one bounded closed JSON document. Exactly one program
variant supplies literal argv or an explicit interpreter/script/login policy.
Environment inheritance and values are explicit; stdin is bounded base64 data.
Invalid/duplicate fields fail before SSH. Public JSON is separate from private
RH01 binary Run frames and the native bounded file JSON protocol.

Preparation creates a private remote namespace with a random generation. Read-only
metadata refuses an absent namespace and never invents one. Mutations bind the
observed generation. Replacing/restoring that namespace invalidates its old
deduplication history; this is an explicit recovery boundary.

Request keys are `<expiry-unix-seconds>-<64-lowercase-hex-random-characters>`.
Expiry is immutable and no more than seven days ahead of effective remote time.
The caller retains its key before submission. Under the registry lock, the
helper persists the maximum of wall time and its clock high-water mark before
registration/pruning. Metadata reads that maximum without rewriting it. A
forward clock jump can expire work early; a pruned expired key stays expired.

The fingerprint covers submitted program, cwd literal, input, environment,
generation, key/expiry and remote deadline. Observation wait is excluded.
The first launch resolves cwd; retry observes that Run rather than launching
against a newly resolved path. Identical concurrent requests register at most
one launch attempt. A changed payload conflicts. Registration fsyncs immutable
intent and accepted evidence before spawning; ambiguous launch never respawns.
This is bounded request deduplication, not exactly-once business effects.

## Run state and control

| State | Evidence |
| --- | --- |
| accepted | durable registration; launch has not published readiness |
| starting | a live owner is preparing the child |
| running | monitor owns the child and can observe/control it |
| exited | durable wait result, either exit code or terminating signal |
| lost | authentic completion/ownership cannot currently be established |

Default wait is zero. Wait expiry, client cancellation and MCP cancellation end
observation; a separate remote deadline or explicit signal controls execution.
A transport error is not a fresh running/lost observation. Preserve stale data
when available and reconnect with the same generation/key. Missing completion
never becomes exit code zero.

The monitor owns a private local control socket and its lifetime lock. Signals
are delivered only by that live owner. It observes leader exit without reaping,
disables control permanently, then reaps and publishes completion. No saved PID
or PGID is signaled after the leader is gone. Children escaping the group are
not contained. Reboot, account-session cleanup or monitor death may yield lost
work; no capability probe guarantees survival under every host policy.

Stdout/stderr have separate append-only retained prefixes. Cursors are absolute
source-byte offsets; base64 preserves exact bytes and displayed UTF-8 does not
change cursor arithmetic. Default pages are 4 KiB, maximum 1 MiB per stream.
Retention is 64 MiB per stream. After quota or write failure the monitor keeps
draining/counting and reports incomplete output; it never invents retained bytes.
Leader completion is independent of EOF. Descendant-held pipes drain for at
most two seconds after leader exit, then close with an incomplete-tail indicator.
Neither leader exit nor EOF proves whole-tree completion.

Pruning is explicit, expiry- and generation-guarded, and retains active/lost
Runs. There is no hidden GC or global storage quota. Remote storage exhaustion
can refuse registration or result persistence; missing evidence remains unknown.

## Helper preparation and trust

The cache identity includes protocol/source, observed OS/architecture and build
profile. An explicit `RHOST_HELPER_BIN` selects its full binary SHA. Explicit
LLVM configuration selects the hash of actual compiler/linker/builtins/sysroot
inputs and fixed flags; a configured failure refuses rather than silently
choosing another build. In automatic mode, reuse a compatible cache, try an
existing remote C99 `cc`, then download the matching pinned release artifact.
No compiler, SDK or package is automatically installed.

`RHOST_NO_UPLOAD` allows matching cache reuse only. `RHOST_NO_SOURCE_BUILD`
disables remote source compilation; an explicitly configured local compiler
remains an explicit choice. `RHOST_NO_DOWNLOAD` disables release downloads.
`RHOST_HELPER_CACHE_DIR` overrides only the private helper cache. Native state
uses `${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost/v5}`; preserved exec/session
operations keep their v4 state locations. Explicit overrides are complete roots.

Bootstrap still needs a working SSH shell and ordinary POSIX tools including
`sh`, `uname`, `id`, `stat`, `cat`, `chmod`, `mktemp` and `rm`. The helper itself
uses OS APIs, not remote Python/base64/setsid/flock command-line programs. Source
compilation additionally needs an existing compatible `cc`. Published fallback
artifacts target static musl Linux x86_64/aarch64; no universal Unix support is
claimed. A native macOS helper is only a local verification fixture.

Private no-follow directories, ownership/mode checks, locks and atomic
fsync/rename protect preparation. A candidate's copied bytes are hashed and its
handshake binds source, target, capabilities, limits and image SHA before
promotion. Cached supplied/release helpers must report their selected full SHA.
Linux reads its loaded image through `/proc/self/exe`; other platforms observe
the checked executable path. This trusts the builder and remote account, not
cryptographic source attestation or protection from the same account replacing
its own files. Never overwrite a running executable in place.

## Context and files

Optional Linux boot ID is an observation, not a hermetic environment handle.
An optional `if_boot_id` guard is validated by the helper after full parsing and
before operation effects; changed/unavailable identity refuses. It does not
enter the immutable payload fingerprint. Preparation may still populate its
private cache before that capability guard runs.

See [targeted changes](file-changes-v5.md) for per-file durable intents, sorted
parent locks and explicit guarded resume/rollback. Global atomic visibility,
nonparticipating-writer exclusion, command rollback and ACL/xattr restoration
are not promised. Search hashes the bytes it matches, with explicit scan/result
budgets and collection-bound cursors; read-many is a set of individual file
observations, not a snapshot.

## MCP and delivery

`rhost mcp` is a stdio server process using the official SDK, without HTTP or an
authentication stack. Tools call the same typed requests as the CLI and expose
operation-specific input/output schemas. At most 16 observations are admitted.
Canceled queued calls cannot later submit work; EOF cancels observers and leaves
Runs alone. The SDK may suppress a canceled call's response; cancellation does
not promise an additional final envelope. Returned tool results use the normal
schema-v3 structured envelope.

## Verification and publication

Helper binaries are compiled locally using `scripts/build-helper-release.py`.
The tracked source-bound lock contains target, ABI, normalized configuration,
compiler/linker/library/sysroot identities and exact binary SHA/size. CI only
fetches and executes those bytes; Rust clients and test harnesses compile in CI.
Native Linux x86_64/aarch64 jobs run direct and temporary-sshd acceptance using
fresh keys and strict known_hosts. Local sanitizers/allocation faults complement
those shipping-byte tests and never replace their artifacts.

The independently reviewed publication design uses a stable non-production
candidate tag, first-attempt Actions artifacts, manual same-commit exact-client
and helper verification, and an annotated production tag binding immutable
artifact IDs. Tag jobs rerun Linux/macOS source gates, all four native client
executions and both native helper jobs without rebuilding the client candidates.
Missing/expired/substituted inputs refuse promotion. The published manual record
is explicitly a trusted operator attestation, not CI-observed real-host evidence.
See [maintenance](../MAINTENANCE.md#v5-publication-procedure).

## Core structured adapters after v5.0.0

Current source adds version/hosts, doctor, connection status and seven session
operations to request/MCP. Their existing schema-v3 result DTOs remain unchanged.
Local discovery rejects host/guard; the added remote operations require a host
and reject boot guards before probing. Both outer input and params must be JSON
objects, preserving duplicate-field refusal in raw JSON input.

Capability-owned application interfaces serve both CLI and structured delivery.
Session observers carry scoped cancellation into actual subprocess waits, without
foreground-exec cleanup on cancellation. Cancellation never itself sends a pane
key and does not revoke an already submitted session timeout. Partial create
identity and missing completion remain available evidence, never safe replay.
