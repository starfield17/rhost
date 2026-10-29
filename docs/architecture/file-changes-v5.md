# Targeted file changes

Status: implemented. Native acceptance and real Linux SSH verification cover
preflight, retries, guarded recovery and structured inspection.

## Boundary

An explicit changeset contains at most 64 write/patch operations on named files.
It owns only the requested bytes, file modes and their recovery journal. Parent
directories must exist. It creates no project, branch, worktree or snapshot and
runs no validation command. Submit a separate Run for validation and explicitly
request rollback if required. Arbitrary command side effects cannot be undone.

The aggregate original bytes and replacement bytes are each bounded to 8 MiB.
Private journal records live in the existing account-owned v5 namespace. No
background service or automatic replay exists. All participating parent locks
are acquired once in a deterministic device/inode order before target checks;
same-directory aliases share a lock. Duplicate entries are rejected by held
parent device/inode plus basename, including textual aliases. All target hashes, regular-file/no-follow
constraints, replacement bodies and modes pass preflight before the first target
replacement. External writers do not participate in advisory locking.

## Identity and journal

Apply requires generation, an expiring caller key and immutable expiry. The key
grammar, durable clock high-water and seven-day bound are shared with Runs.
The canonical request fingerprint binds every requested change. Register and
fsync that request before preflight; a changed retry conflicts, and an identical
retry only observes the existing record. It never silently resumes mutation.

Lock order is namespace registry, per-changeset operation lock, then sorted
parent locks. Acquire transaction ownership under the namespace lock, release
the namespace lock before parent locks, and never reacquire it while holding
transaction/parent locks. Apply/recover reload the journal after taking ownership.
Query reads atomic records without claiming an intent marker means abandonment.
Prune takes the same operation lock nonblocking and retains busy records.

Persist originals, intended replacements and per-file versions before applying.
Each same-directory staging inode and its intended hash/mode are recorded and
fsynced before its target can change. A version contains device/inode, size,
mode and nanosecond timestamps. Each application has a durable intent marker,
an atomic rename/non-clobbering link, parent fsync, and a durable result marker.

| Recorded phase | Observation | Permitted explicit recovery |
| --- | --- | --- |
| preflight | no target application was authorized | redo preflight or stop |
| prepared/pending | original version still matches | resume |
| installing | recorded staged inode/hash/mode is now at target | reconcile to applied |
| installing | original version still matches | resume |
| installed | recorded installed version still matches | rollback or retain |
| rolling back | recorded rollback inode/hash/mode is at target | reconcile to restored |
| any | neither guarded version matches | conflict/unknown; preserve current file |

An ambiguous observation is not proof of absence of earlier effects. Recovery
reports observed versions and mutation evidence; it does not claim exactly-once
history. For an interrupted install, staging inode, content, mode and unchanged
mtime identify the intended object; a normal completed marker additionally binds
ctime. Differing observed revisions are refused. Advisory locks cannot exclude a
nonparticipating write between comparison and rename/unlink.

## Recovery and observation

Query is read-only with respect to target files. Apply/query/recover return a
record with per-file states and aggregate mutation evidence. Apply/recover errors
retain that data; `ok:false` cannot discard a partial journal. Capability-owned
`files::RequestOutcome` carries data plus an optional shared Error to the request
composer; there is no files-to-runs or files-to-request dependency.

Resume is explicit, reuses archived replacement bytes, and validates all
remaining originals plus already installed versions before further mutation.
Rollback preflights all installed versions, then restores only guarded files in
reverse order with the same durable per-file protocol. A new file is removed
only while its installed inode/version still matches. A conflicting external
edit is retained and reported. Missing/corrupt evidence is never treated as an
absent transaction or permission to overwrite.

Query and explicit recovery remain available after expiry while the journal
exists. Expiry prevents new registration, not recovery.

Staging names and inode identities are journaled. Cleanup occurs only under
transaction ownership and checks the staging inode. A cleanup failure retains
the record for safe retry rather than dropping its recovery evidence.

Pruning is explicit, generation guarded and expiry bounded. Only terminal
applied/restored/refused records may be pruned; partial/unknown journals remain.
Pruning deliberately ends the recovery window. An expired key cannot register
again even after pruning. There is no global atomic visibility to external
readers and no automatic rollback after network failure.

## Additional host observation

Native metadata reports helper source/protocol/target/capabilities and optional
Linux boot ID. An optional structured-request boot guard is checked by the native
helper after full request validation and immediately before capability effects.
The guard is an observation precondition, excluded from immutable launch/change
fingerprints; changing it cannot authorize a different payload under a key. Unavailable or changed identity
refuses; omission makes no pinning claim. Private preparation may still create
its isolated helper cache. Environment values remain explicit Run request data,
not a claim of hermetic execution.
