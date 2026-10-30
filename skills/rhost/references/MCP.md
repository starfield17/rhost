# MCP setup and structured requests

The local `rhost mcp` process serves stdio MCP and launches system OpenSSH for
remote operations. The MCP client owns this local process; remote files, Run
monitors and tmux own persistent work.

## Connect a client

Install the CLI first. Its process needs the intended OpenSSH configuration,
trusted host keys and authentication agent. A GUI launcher can have a different
PATH or `SSH_AUTH_SOCK` from your terminal; use an absolute executable path when
resolution differs. Establish OpenSSH trust/authentication beforehand: rhost
does not prompt for credentials or disable host-key checking.

The portable launch definition is:

```json
{"command":"rhost","args":["mcp"]}
```

For Codex, register and inspect the server:

```bash
codex mcp add rhost -- rhost mcp
codex mcp list
```

Equivalent Codex configuration:

```toml
[mcp_servers.rhost]
command = "rhost"
args = ["mcp"]
```

For Claude Code, local scope applies to the current project for the current user:

```bash
claude mcp add --transport stdio --scope local rhost -- rhost mcp
claude mcp get rhost
```

See official [Codex configuration](https://developers.openai.com/codex/mcp) and
[Claude Code configuration](https://code.claude.com/docs/en/mcp) for locations
and scopes. These registration commands write configuration when the user runs
them. Installing the skill/plugin does not register MCP automatically; preserve
unrelated client settings.

## Discover and call tools

Use the connected server's tool list; clients may add a server prefix to names.
v5.2.0 uses schema 4, adds state discovery and path collections, and provides local discovery, diagnostic and session tools alongside the
native helper/Run/file tools available in v5.0.0. Older clients expose a smaller
set; use advertised tools or the corresponding CLI operation.

| Tools in this checkout | Arguments and purpose |
| --- | --- |
| `rhost_version`, `rhost_hosts` | Empty params, no host/boot guard; local build and best-effort alias discovery. |
| `rhost_doctor` | host, params with optional timeout_ms/fresh; shell capability diagnosis. |
| `rhost_connection_status` | host, empty params; shared OpenSSH master evidence. |
| `rhost_helper_prepare`, `rhost_metadata` | host, empty params; native preparation or read-only namespace observation. |
| `rhost_run_*` | [RUNS.md](RUNS.md): recoverable execution and separate byte streams. |
| `rhost_fs_*` | [FILES.md](FILES.md): observations, CAS edits and named recovery. |
| `rhost_state_*` | Explicit state discovery, usage and prune preview; [FILES.md](FILES.md#state-discovery-and-retention). |
| `rhost_session_*` | [SESSIONS.md](SESSIONS.md): persistent terminal interaction. |

For example, `rhost_fs_read` takes:

```json
{"host":"gpu","params":{"path":"~/work/project/config.toml","max_bytes":4096}}
```

`structuredContent` is the schema-v4 envelope. Inspect operation-specific
completion and partial-result fields. A successful Run observation can contain
a failed child exit; an error can still contain effects or useful data.

Doctor/session tools do not require native helper preparation. Doctor reports
command presence and checks its state directory's writability. `ok:true` does
not mean all optional capabilities exist; native readiness is a separate check.

Foreground exec, transfers, tunnels, connection reset and audit queries remain
CLI-only. MCP session exec takes inline `command`; local command-file loading
and streaming/pipelines belong to CLI. `session attach` remains refused.

## One JSON request without MCP

`rhost request --from <file|->` reads one document, up to 16 MiB, and emits JSON:

```bash
rhost request --from - <<'EOF'
{"schema_version":4,"operation":"fs.read","host":"gpu","params":{"path":"~/work/project/config.toml","max_bytes":4096}}
EOF
```

Outer fields are schema_version, operation, params, plus host for remote
operations. Version/hosts forbid host. Both the document and params are objects;
unknown/duplicate fields and malformed inputs fail before external submission.
It is one operation, not a general batch/DAG language. MCP arguments omit
schema_version and operation because the selected tool already identifies them.

Only native helper/Run/file operations accept optional `if_boot_id`. Changed
or unavailable Linux boot identity refuses capability effects; private helper
cache preparation may still occur. Other operations reject the guard before
probes. Omitting it makes no pinning claim.

## Cancellation and context

Ordinary calls have 16 active slots and 32 queue places (32 MiB queued arguments).
Control calls (`run.signal`, session recover/close) have two independent slots
and eight queue places (4 MiB). Local discovery has its own two/eight capacity.
`MCP_BUSY` means bounded admission refused before submission. Canceled queued work
never submits later. Control still obeys remote ownership locks and network limits.
Cancellation/EOF stop observers, not Runs or terminal panes; an already submitted
session timeout may still interrupt if its helper continues. The SDK may suppress
a canceled call's reply, so retain identity and inspect through a new call.
Session input has no Run deduplication guarantee.

Read only the workflow reference needed now. Skill disclosure and MCP tool
discovery are separate: clients control eager versus deferred tool definitions.
See [Claude Code tool search](https://code.claude.com/docs/en/mcp#scale-with-mcp-tool-search)
and Codex's tool filters. Connecting MCP does not imply zero context cost.

## Select tool groups

Launch `rhost mcp --tools runs,files` to expose only selected groups; options are
`runs`, `files`, `sessions`, `diagnostics`, or default `all`. Base version, metadata,
helper preparation and state-management tools remain available. The same filter
rejects calls to hidden tools. Restart/reconnect to change the selection.

All tools carry conservative behavior annotations. Native reads may prepare a
helper/cache, so their annotations do not promise a side-effect-free environment.
Annotations are hints, not authorization. Results retain both structured JSON and
serialized text as recommended by the MCP tools specification. Wire-byte savings
from specialized schemas do not establish a particular client's token savings.
