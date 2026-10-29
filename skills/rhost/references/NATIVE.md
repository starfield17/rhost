# Native Runs, requests and MCP (v5)

All public JSON uses schema v3. Use `rhost version --json` to check the installed
binary. Existing `exec` remains foreground with its existing timeout/cleanup
contract; sessions retain interactive terminal state. Use native Runs for
noninteractive work requiring durable completion and separate output streams.

## Prepare and retain identity

```bash
rhost helper prepare gpu --json
rhost helper status gpu --json
```

Preparation returns `data.generation`, `data.remote_time_unix`, optional
`data.context.boot_id`, and `data.helper` with source/target/protocol/limits and
actual executable SHA. Status is read-only and refuses an absent namespace.
Doctor's command-presence map is separate from native readiness.

Generate a random key before submission:
`<expiry-unix-seconds>-<64-lowercase-hex-random-characters>`. Derive expiry from
observed remote time, at most seven days ahead, and retain generation/key/payload
before submitting. Namespace replacement or restoring old state invalidates
deduplication history. Expired keys cannot launch again after pruning.

```bash
rhost run start gpu --generation '<generation>' --key '<expiring-key>' \
  --cwd '~/work/project' --argv-json '["/bin/sh","-c","printf finished"]' \
  --wait 10s --deadline 30m --json
rhost run poll gpu '<run-id>' --generation '<generation>' \
  --wait 30s --stdout-since 0 --stderr-since 0 --max-bytes 4096 --json
rhost run signal gpu '<run-id>' --generation '<generation>' --sig TERM --json
rhost run list gpu --generation '<generation>' --json
rhost run prune gpu --generation '<generation>' --json
```

`--wait` ends observation; it never stops a Run. Its default is zero, and an
accepted/starting result is valid before readiness. `--deadline` is enforced by
the remote monitor. Signals are explicit `INT`, `TERM` or `KILL`. Canceling the
local CLI or MCP observer leaves remote work alone.

Use absolute program paths and explicit `--env KEY=VALUE` where reproducibility
matters. Environment inheritance defaults to false; `--inherit-env` explicitly
inherits the SSH invocation environment. Shell programs use `--command`, an
absolute `--shell` (default `/bin/sh`), and optional `--login`. No profile is
implicitly loaded for argv execution. This is an explicit environment contract,
not a hermetic environment guarantee.

## Observation and retry

Inspect `data.run.status`: `accepted`, `starting`, `running`, `exited` or `lost`.
Only `exited` has `data.run.completion.exit_code` or `.signal`. `ok:true` means
the observation succeeded, including a nonzero child exit; inspect completion.

`data.stdout` and `data.stderr` each contain `from`, `next`, `observed_bytes`,
`content_base64`, displayed `content`, `open` and `incomplete`. Cursors count
source bytes. Repeat a retained offset to get the same bytes; advance with
`next`. Do not derive offsets from text length. Retention is 64 MiB per stream;
quota/write failure keeps draining/counting but reports gaps. Leader exit and
stream EOF are separate facts. Descendants are not a contained process tree.

An unknown submission is safely *deduplicated* only by the same still-valid
generation/key/payload. A changed payload returns `IDEMPOTENCY_CONFLICT`.
After expiry, poll the existing Run rather than trying to resubmit it. Never
turn `lost`, SSH failure, missing output or a vanished process into success or
permission to start a second side effect. Signal refuses without a live owner.
Prune explicitly removes only eligible expired completed records and ends their
retention window. There is no automatic restart, scheduler or background GC.

## One structured request

`request --from <file|->` reads one bounded document and always emits JSON.
Commands are data inside JSON, avoiding the local shell parser. For example:

```bash
rhost request --from - <<'EOF'
{"schema_version":3,"operation":"fs.read","host":"gpu","params":{"path":"~/work/project/config.toml","max_bytes":4096}}
EOF
```

The closed outer fields are `schema_version`, `operation`, `host`, `params` and
optional `if_boot_id`. Use a valid observed Linux boot ID to refuse drift;
changed/unavailable identity fails before capability effects. Private helper
preparation may still populate its cache. Omitting the guard makes no pinning
claim. Unknown/duplicate fields fail before SSH.

`run.start` params contain `generation`, `key`, matching integer `expires_unix`,
`cwd`, `program` (`argv`, or `interpreter`/`script`/`login`), and
`environment` (`inherit`/`values`). Optional `stdin_base64`, `wait_ms` and
`deadline_ms` provide explicit input/budgets. Program stdin is data in the request,
never bytes following the JSON document. Other operations use their own closed
parameters, advertised by MCP and the published request schema.

## Structured files and explicit recovery

`fs.search` params: an explicit `root`, optional POSIX `glob`, single-line
literal `pattern`, result/match/text limits and collection-bound `cursor`.
`data.items[]` returns file paths, full hashes and line matches from the same
observed bytes. `matches_truncated` means that file's match list was shortened;
the next cursor advances beyond that file. Scans are bounded by entries/depth,
8 MiB per file and 64 MiB actual read bytes. Check `incomplete`, `reasons` and
`skipped`; incomplete scans have no continuation. Symlinks are excluded. The
glob uses POSIX fnmatch over relative paths; `*` can cross `/` and match dots,
and `**` has no special recursive meaning.

`fs.read_many` takes up to 64 explicit `paths`, optional line range and one
aggregate `max_bytes` budget. `data.items[]` preserves input order with per-file
results/errors; `unread` names paths omitted by budget. Each hash is a file
observation, not a workspace snapshot.

`fs.changeset.apply` takes generation/key/expiry and up to 64 named `changes`.
Each is a `write` with UTF-8 `content`, optional octal `mode`, and `if_hash` for
existing files; or a `patch` with `if_hash` and edits. Omit `if_hash` only to
create a missing file. Aggregate originals/replacements are each at most 8 MiB.
All targets pass preflight before replacement. Parents must already exist.

Use `fs.changeset.query` with generation/`id` to inspect. Identical apply retries
only observe; they never silently resume. `fs.changeset.recover` requires
explicit `action: "resume"` or `"rollback"`. Recovery validates installed/original
versions and preserves later external edits. Read status, per-file states,
`mutation` and `cleanup_pending` even on `ok:false`. Mutation records historical
forward application; rollback does not erase that history. Pruning explicitly
ends recovery for an eligible expired terminal journal.

CLI views use `rhost fs changeset <apply|query|recover|prune> gpu --from params.json`,
`rhost fs search gpu --from params.json` and `rhost fs read-many gpu --from params.json`.
Their input is the operation's params object. Request/MCP use the same semantics.

There is no global multi-file atomic visibility or automatic rollback. Run
validation separately and explicitly request recovery. rhost creates no project,
branch, worktree or snapshot; manage those through your own chosen tools.

## MCP

Start `rhost mcp` as a stdio server in an MCP client's server configuration.
Tools such as `rhost_run_start`, `rhost_run_poll`, `rhost_fs_patch` and
`rhost_fs_changeset_apply` accept `host`, operation-specific `params` and an
optional `if_boot_id`. They call the same typed requests without a local shell.
Returned `structuredContent` is the schema-v3 envelope. At most 16 observations
are admitted; canceled queued work is not submitted later. EOF stops observers.
The official SDK may suppress replies to canceled calls; poll the Run afterward.

## Preparation policies

An explicit `RHOST_HELPER_BIN` selects exact local bytes. Alternatively,
`RHOST_HELPER_CC`, `RHOST_HELPER_SYSROOT`, `RHOST_HELPER_BUILTINS` and
`RHOST_HELPER_LD` select a fixed local LLVM/musl build for the observed Linux
architecture. Invalid/failed explicit configuration refuses without fallback.
Automatic preparation tries existing remote C99 `cc` and then a SHA-pinned
static Linux release binary. No packages or toolchains are automatically installed.

`RHOST_NO_UPLOAD` allows compatible cache reuse only; `RHOST_NO_SOURCE_BUILD`
disables remote source builds; `RHOST_NO_DOWNLOAD` disables release downloads.
`RHOST_HELPER_CACHE_DIR` chooses a private cache root. Neither the native helper
nor its operations require remote Python/tmux. Bootstrap still needs SSH's shell
and ordinary `sh`, `uname`, `id`, `stat`, `cat`, `chmod`, `mktemp` and `rm` tools.
