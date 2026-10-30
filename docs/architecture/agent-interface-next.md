# Agent interface development contract

Status: planned for the local 5.2.0-dev.0 development tree. This is not a
published release. Schema 4 replaces schema 3 for public delivery; the published
v5.1.0 schemas and fixtures remain immutable. Existing v5 Run and changeset
records remain readable and recoverable. Project directories and Git remain
caller-owned.

## Admission and delivery

MCP has independent ordinary, control and local-discovery admission lanes.
Ordinary execution admits 16 calls and 32 queued calls with a 32 MiB aggregate
queued-argument budget. Control (`run.signal`, `session.recover`, `session.close`)
admits two calls and eight queued calls with a 4 MiB budget. Local version/hosts
admits two calls and eight queued calls with a 4 MiB budget. These limits bound
admitted work, not SDK parsing buffers or total process RSS. A full queue returns
`MCP_BUSY` before capability submission. Queue bytes and permits have RAII owners.
Cancellation/EOF is checked before admission and at the capability boundary;
cancelled queued work cannot submit later. Control admission never bypasses
remote ownership or registry locks and cannot guarantee network availability.

`mcp --tools all` is the default. A comma-separated selection from `runs`,
`files`, `sessions`, `diagnostics` restricts both discovery and invocation.
Version, native metadata and helper preparation form the base group. Tool
annotations are conservative behavior hints, not access control. Specialized
schemas preserve the authoritative validator's acceptance, including closed
objects and failure shapes. Structured results retain their serialized text
counterpart. Measured wire bytes do not imply a particular client's token cost.

## Native dispatch

A warm operation opens one SSH channel. Within that invocation, bootstrap
selects the observed OS/architecture and exact preparation profile, checks the
private cache, and enters a verified helper mode. Before capability dispatch,
the loaded helper checks its source, target, capabilities, limits and any pinned
full image SHA, then emits a handshake prologue. The operation's answer follows.
This is one verified operation, not a persistent remote service.

Only a typed cache-miss or verified pre-dispatch incompatibility permits
preparation followed by submission. A transport error, truncated handshake,
unrecognized executable response or malformed frame never authorizes replay.
Supplied/release profiles bind their exact SHA; configured LLVM profiles retain
the hash of actual compiler/linker/builtins/sysroot inputs. A cached target or
path is only a hint and never replaces remote verification.

## Discovered-path collections

`fs.find` discovers regular-file paths without reading contents. `fs.search`
matches literal text and hashes exactly the observed matching bytes. Both accept
explicit root-relative `exclude_dirs` before descending or charging traversal
entries. There are no implicit project/Git ignore rules.

The first page builds an immutable discovered-path collection, not a filesystem
or directory snapshot. Directories are observed at different times. Collection
limits are 100000 paths, an 8 MiB path manifest, depth 32 and 100000 visited
entries. Traversal incompleteness and bounded skipped-path diagnostics remain
explicit on every page. Content reads are bounded independently to 8 MiB/file
and 64 MiB/page. Matches/result/text budgets remain explicit.

Each collection has a random expiring ID in the private v5 namespace. Its
fingerprint binds operation, normalized root, glob, literal pattern and sorted
exclusions; page size/budgets are not fingerprint fields. Default lifetime is
600 seconds, maximum 3600, against the durable effective remote clock. A cursor
binds generation, collection ID and the next manifest record byte offset.
Each path record has a checksum bound to its query identity, ordinal and offset.
Stored paths must have canonical root-contained components. Index and record
boundaries are checked locally to each page. Offsets must be valid record boundaries; IDs, fingerprint, manifest size and
regular-file ownership are validated before reading a page. Expired, missing or
corrupt collections refuse continuation; they never silently start a new search.

Registration under the namespace lock first reserves a private directory named
`.pending-<expiring-id>` in its kind-specific parent; its kind, identity and
expiry remain recoverable even before metadata or owner.lock exists. The
registry lock is retained until the owner lock has been acquired and made
durable, so missing metadata/lock in a pending directory cannot hide a live
builder from a later registry owner. Registration then
fsyncs its `building` metadata plus owner lock. Traversal occurs under that
record's exclusive owner, after releasing the namespace lock. A complete
manifest and metadata are fsynced before atomic publication as `ready`.
Construction failure publishes `failed` when possible. An ownerless building
record remains incomplete; it is never reused as a ready collection. Queries
are read under the same record lock used by pruning; expiry alone does not
delete a record or release capacity. At most 32 collections may be retained.

Later pages seek into the manifest and read only newly visited file contents.
Content changes receive fresh per-file hashes without invalidating traversal.
Removed/unreadable paths are reported and traversal advances. New paths require
a new collection. A content budget stops before an unread file and returns its
continuation; it must not silently skip that file. Per-file match truncation
remains distinct from traversal incompleteness.

## State discovery, capacity and pruning

`run.list` and `fs.changeset.list` return generation, items and a nullable next
cursor. Pages default to 64 records and cap at 256. Cursors bind generation,
record kind and last ID. Listing observes each record independently, includes
per-item evidence errors and never treats one corrupt record as an empty list.
Concurrent additions need a new traversal to guarantee discovery; lists are not
snapshots. Existing namespaces over the admission limits remain inspectable,
recoverable and prunable.

`state.usage` reports logical file bytes, reserved bytes, record/query counts
and admission limits. It does not claim filesystem blocks or free disk space.
Admission is bounded to 16384 records and 8 GiB reserved logical state. Under
the registry lock, reservations are derived from retained records rather than
a separately mutable counter:

| Record | Charged reservation |
| --- | --- |
| Run with active owner or nonterminal/lost evidence | max(actual, 145 MiB) |
| Exited Run with unowned lifetime lock | actual |
| Active or nonterminal changeset | max(actual, 33 MiB) |
| Terminal unowned changeset | max(actual, 33 MiB) |
| Any retained collection, including expired/failed/building | max(actual, 10 MiB) |

Run bounds cover both immutable request/stdin copies, both 64 MiB streams and
metadata. Legacy v5 monitors obey these same bounds. Exited leader evidence alone
cannot release a reservation while the monitor can still drain output. Changeset
bounds cover request, originals, replacements and metadata. Every retained
changeset keeps its full reservation because explicit recovery can allocate
again even from a refused/applied terminal state. All pending/staging
directories are charged. Unknown/corrupt accounting refuses new admission and
reports incomplete usage; it never guesses zero. Integer overflow refuses.
No reservation is released from a lost ownership claim. New registration fails
with `STATE_CAPACITY_EXCEEDED` before registration or target mutation when its
reservation would exceed a limit. No background GC exists.

`state.prune` names one kind and up to 64 explicit IDs, with `dry_run:true` by
default. It returns one bounded receipt per ID: eligibility, reason, deletion
result and optional error. Eligibility requires expired Run completion plus an
unowned monitor lock, an expired terminal changeset with no pending cleanup, or
an expired unowned collection. An expired unpublished pending registration with
no active owner is also eligible; its identity survives missing metadata. Active/lost Runs and partial/unknown changesets
are retained. The durable expiry clock is advanced before deletion authorization.

Prune acquires namespace then record ownership without waiting while holding
another record lock. It durably renames an eligible record to a private pruning
tombstone before removing its bytes, retaining the ID/expiry in the tombstone
name. Failed deletion is reported and the tombstone remains discoverable for
explicit retry; it is still charged. Tombstones never become fresh executable
registrations, and an expired key remains expired. A partial batch returns its
receipts, not an empty error. Dry runs do not delete, advance the durable clock,
or alter record state. Pruning deliberately ends the named retention window.
Tombstone deletion is serialized by the registry lock and its kind/ID/expiry
remain in its name even after metadata and owner.lock have been removed. Retry
finishes deletion from that identity rather than recreating executable state.
Existing run/changeset prune operations use the same tombstone deletion primitive;
there is no alternate unlink-in-place path. Legacy staging names whose identity
cannot be established remain explicitly unknown and refuse new admission rather
than becoming safe-to-delete evidence.

## Recovery evidence

Every public error has a `recovery` object with `stage`, `submission` and
`action`. Stages are validation, preparation, submission, observation, operation
or unknown. Submission is not_submitted, may_have_submitted or confirmed;
confirmed means submission evidence, never successful completion.
Actions are correct_request, retry_request, retry_observation, retry_same_request or
inspect_state. Unsupported evidence defaults to unknown/may_have_submitted/
inspect_state. Advice is independent of `retryable`.

Preparation failures distinguish private helper/cache effects from capability
submission: not_submitted concerns the requested capability only. Validation and
typed pre-dispatch refusal carry not_submitted. A read-only observation may be
repeated. Only caller-retained immutable Run/changeset intent can carry
retry_same_request; that advice only rediscoveries/deduplicates and never
restarts lost work. Expired keys direct the caller to observation. No English
message is parsed to determine recovery.

## Verification

Contract/schema/acceptance changes land independently before implementation.
Coverage includes saturated control admission, bounded count/bytes and cancelled
queues; single-channel warm operations and substituted-image refusal; excluded
large trees, fresh hashes, continuation without previous-content reads, expired
and corrupt collections; discovery beyond 4096 records, partial prune receipts,
crash/legacy reservation accounting and capacity refusal before effects; typed
recovery across preparation/submission failures; filtered tools and equivalent
smaller schemas. Exact helper binaries are built locally and verified through
real Linux SSH, with sanitizer/allocation-failure complements. No helper is compiled in CI. Formal v5.2.0 publication follows the existing
exact-candidate procedure after all required evidence passes.
