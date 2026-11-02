# Context-bounded observation (v5.3.0)

Status: implemented. Exact candidate publication evidence is recorded by the
release and operator-attestation workflows, separately from this source document.
Public JSON advances to schema 5. Published schema 4 validators and fixtures
remain frozen. Existing Run and changeset records remain usable.

## Owners and consumers

This is Risk A: CLI scripts, structured request callers, MCP harnesses, validators,
helper codecs and session protocols are affected. Capability-owned validated
requests, outcomes and DTOs own defaults, coverage and continuation. CLI and MCP
adapt delivery only. Run stdout/stderr remain distinct from session PTY content.
C99 remains the helper language; its implementation and exact artifacts change.
No durable state moves into a client, watcher or adapter process.

## Reading and coverage

File pages default to 8192 content bytes. Preserve explicit maximum budgets.
Each page identifies its whole-file hash, delivered byte range, remaining ranges,
limit reason and an executable continuation request. A continuation pins the
observed hash and refuses a changed file. UTF-8 fragments never invent offsets.
An oversized line can be split at a character boundary; a successful nonempty
page must advance. Reading the requested line count does not imply file EOF.
Read-many retains an aggregate content budget, per-file errors and unread paths.

Search has separate path-discovery and pending-file positions. A budget stopping
within one file must allow the remaining matches to be retrieved before moving
past it. The pending-file cursor pins that file's hash; later, unvisited files
remain fresh observations. Finished files are not reopened for path continuation.
Oversized matching lines expose bounded previews plus exact file-read ranges.

## Run and terminal observation

Add run.watch, run.read and run.search to all three delivery forms. Watch returns
state, completion and counters without log bodies, waiting at most wait_ms
(default 30000, maximum 300000). Cancellation stops observation, never the Run.
Read defaults to a 4096-byte retained-tail preview of an explicit stream, with
range continuation. Text is the default representation; base64 is explicit.
Search uses bounded literal matches and context, byte positions and a fixed
observed endpoint. Existing run.poll continues to use shared Run primitives.

Omitted ranges are not acknowledged or consumed. Existing retention limits stay
in force: a retained-prefix tail must never claim to be the unretained real tail.
Unavailable bytes, open streams and incomplete evidence remain explicit.
Completion never means all logs have been read or retained.

Session read defaults to 4096 bytes and supports an upper byte boundary. Session
exec accepts an explicit output budget (default 4096, existing maximum retained)
and exposes the exact completed command segment plus a bounded tail and read
continuation. Unknown submission/completion never invents a command boundary.

## Evidence and publication

Acceptance must reconstruct long UTF-8 lines, find a failure in the middle of
large logs, refuse changed sources, preserve canceled observers and distinguish
completion from unread/unavailable bytes. Fixed fixtures must produce equivalent
business results through CLI, request and MCP. Measure actual Codex CLI input,
cache and call counts; wire bytes alone are not token evidence.

Run complete local gates and native helper sanitizer/fault complements, then
verify exact candidate clients/helpers with test-live-all and test-live-helper.
Publish v5.3.0 only through the existing immutable-candidate and operator-evidence
workflow. Targets, private paths and real transcripts remain untracked.

## Codex CLI measurement

Measured with Codex CLI 0.166.0, schema 5 and the configured model
`gpt-6.1-sol`, low reasoning effort, an empty working directory, ephemeral
sessions and isolated per-invocation MCP configuration. No global configuration
was changed. These are single observations, not a controlled token benchmark;
Codex reports cumulative turn usage, including discovery and cached input.

| Task | MCP selection | Input tokens | Cached input tokens | Completed MCP calls |
| --- | --- | ---: | ---: | ---: |
| Return OK without tools | none | 15011 | 12288 | 0 |
| Return OK without tools | all | 14984 | 12288 | 0 |
| Return OK without tools | runs,files | 15929 | 12288 | 0 |
| Read binary/schema version | all | 48974 | 44544 | 1 |
| Read binary/schema version | runs,files | 49017 | 44416 | 1 |

Both version calls used the actual MCP tool and returned binary 5.3.0/schema 5.
This client uses deferred discovery; this sample does not establish a token
reduction from server-side filtering. Tool groups remain useful for limiting
exposed capabilities and for clients that eagerly load definitions. The
verified context improvement is bounded returned content and recoverability:
8192-byte file defaults, 4096-byte Run/session previews, body-free watch,
selected streams and explicit binary encoding. Explicit old-sized budgets are
still available; larger files require intentional continuation calls. Wire JSON
retains text plus structured content for MCP compatibility and must not be
reported as the client's actual token count.
