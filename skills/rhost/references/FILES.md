# Files: observations, CAS edits and explicit recovery

Use available MCP tools or one JSON request for structured operations. CLI file
operations use the same native helper; remote Python is not required. Helper
preparation is automatic. For preparation policy failures, read
[NATIVE.md](NATIVE.md#preparation-policies).

`fs.search` and `fs.read_many` can supply hashes for subsequent CAS edits.
Read the actual content needed for a change, and check partial results before
assuming a search or collection is complete.

## CLI editing


```bash
rhost fs read <host> '<remote-file>' --lines 200 --start 1 --json
rhost fs write <host> '<remote-file>' [--from ./file] [--if-hash <sha256>] [--parents] [--mode <octal>] --json
rhost fs patch <host> '<remote-file>' [--patch ./patch.json] [--if-hash <sha256>] --json
```

Quote a leading `~` so the local shell does not expand it.

`fs read` returns bounded UTF-8 text and the complete file's SHA-256. Creating a
new file needs no hash. Replacing an existing file and every patch require the
fresh hash returned by the last read.

`fs read` defaults to `--start 1 --lines 200`. `--max-bytes` defaults to 256 KiB
and is capped at 8 MiB; an out-of-range CLI value is refused as `USAGE_ERROR`.
`fs write --mode` is three or four octal digits;
when it is omitted a newly created file is `0600` and a replacement keeps the
existing file's permissions. Editing names an exact regular file and refuses a
symlink in any path component; `fs sync --delete` is separate because it resolves
the destination before applying its destructive-safety checks.

### Patch document

`fs patch` takes JSON line-range replacements, not a unified diff. After
`fs read --json`, copy `data.sha256` into the document below in place of
`<sha256-from-read>`. For an original file containing `alpha\nbeta\n`, this
changes the second line to `gamma`:

```json
{
  "sha256": "<sha256-from-read>",
  "edits": [{"start": 2, "end": 2, "text": "gamma\n"}]
}
```

Save it as `patch.json`, then apply it:

```bash
rhost fs patch gpu '~/work/example.txt' --patch ./patch.json --json
```

Ranges are 1-based inclusive lines in the original file, must not overlap, and
must stay inside that file. Text is inserted literally, including its newlines;
an empty string deletes the selected lines. An explicit `--if-hash` overrides
the document's hash. On `FILE_CONFLICT`, read and merge again before applying.

### Writing and patching via stdin (Heredocs and pipes)

Both `fs write` and `fs patch` read from stdin by default (`--from -` and `--patch -`
are implicit when omitted) with an 8 MiB limit. Agents do not need to create local
scratch files:

```bash
# Stdin one-liner to write or create a remote file:
printf 'line 1\nline 2\n' | rhost fs write gpu '~/work/example.txt' --parents --json

# Multi-line write without local files:
rhost fs write gpu '~/work/example.txt' --parents --json << 'EOF'
line 1
line 2
EOF

# Stdin one-liner hash-guarded replacement:
printf 'line 1\nmodified line 2\n' | rhost fs write gpu '~/work/example.txt' --if-hash <sha256-from-read> --json

# Multi-line hash-guarded replacement (safe compare-and-swap):
rhost fs write gpu '~/work/example.txt' --if-hash <sha256-from-read> --json << 'EOF'
line 1
modified line 2
EOF

# Multi-line hash-guarded patch (line-range):
rhost fs patch gpu '~/work/example.txt' --json << 'EOF'
{
  "sha256": "<sha256-from-read>",
  "edits": [{"start": 2, "end": 2, "text": "gamma\n"}]
}
EOF

# Exact-text replacement patch (recommended for coding agents):
rhost fs patch gpu '~/work/example.txt' --json << 'EOF'
{
  "sha256": "<sha256-from-read>",
  "edits": [{"old": "modified line 2\n", "new": "gamma\n"}]
}
EOF
```

A patch cannot insert into an empty file; use a hash-guarded `fs write` for that replacement.

## Structured editing

MCP `rhost_fs_patch` takes `host` and `params` containing `path`, `if_hash` and
`edits`. The CLI patch document above uses `sha256`; these input shapes are
different. Use the advertised tool schema rather than pasting a CLI document
into `params`.

```json
{"host":"gpu","params":{"path":"~/work/project/config.toml","if_hash":"<sha256-from-observation>","edits":[{"old":"debug = true","new":"debug = false"}]}}
```

`fs.write` takes UTF-8 `content`, not a local file path. Omit `if_hash` only for
a missing file. CLI and structured edits return the new hash. Inspect mutation
evidence even on failure; see [RECOVERY.md](RECOVERY.md).

## Search, read-many and named changesets


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

