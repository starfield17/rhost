# Persistent terminals

Use sessions for a shell that retains cwd/environment, a REPL, or a debugger.
Builds and tests needing durable completion belong to [Runs](RUNS.md).
Sessions require remote bash, tmux and flock; they do not require the native C
helper. Use doctor when those dependencies need diagnosis.

## MCP interaction

Available tools are `rhost_session_create`, `rhost_session_list`,
`rhost_session_exec`, `rhost_session_send`, `rhost_session_read`,
`rhost_session_recover` and `rhost_session_close`. Every call takes `host` and
operation-specific `params`. Use the canonical ID returned by create afterward.

For a REPL, create a session, send its launch command with `enter: true`, then
use send/read. Do not wait for a REPL through session exec: exec waits for a
shell completion boundary and its timeout may interrupt the program.

```json
{"host":"gpu","params":{"name":"debug","cwd":"~/work/project"}}
```

The create arguments above are followed by a send call such as:

```json
{"host":"gpu","params":{"session":"<session-id>","data":"python3 -i","enter":true}}
```

Send exactly one nonempty `data` or supported `key`; `enter` belongs only to
data. For read, use `session`, `since` or `tail`, and `max_bytes` (4096 is a useful
small page). Continue from `data.next` while `data.more` is true. Cursors count
raw bytes, including terminal sequences; displayed text length is not a cursor.
`SESSION_BUSY` means use send/read; recover explicitly interrupts the pane.

MCP cancellation/EOF stop the observer without sending Ctrl-C or closing the
session. An already submitted exec timeout is not revoked; if its remote helper
continues, that timeout may still interrupt the pane. Inspect with a new read or
list before repeating input. Sessions do not have Run idempotency keys.

Structured `timeout_ms` defaults to 60 seconds for create/exec and 30 seconds
for list/read/recover; zero selects the default. Send/close have a fixed
30-second operation budget. Exec accepts inline `command`; loading a local
command-file belongs to CLI. These tools do not support `if_boot_id`.

## CLI and result fields


```bash
rhost session create <host> --json --name debug --cwd '<remote-directory>'
rhost session list <host> --json
rhost session exec <host> debug --command '<shell-program>' --json
rhost session send <host> debug --data 'python3 -i' --enter
rhost session send <host> debug --key C-c
rhost session read <host> debug --json --since 0
rhost session recover <host> debug --json
rhost session close <host> debug --json
```

`session create` accepts `--name`, `--cwd`, `--shell bash`, and `--timeout`.
`session exec` accepts `--command` or `--command-file` (mutually exclusive), and
`--timeout`. A command file is a regular local UTF-8 file, not a remote path;
the same 64 KiB limit and file checks as foreground exec apply.
List, read, and recover also accept their documented timeout. Writers are
serialized remotely. `SESSION_BUSY` means another program owns the pane and the
requested command was not submitted. `send --data` is verbatim; `--enter` pastes
the data and presses Enter in the same submission.

Session exec returns `data.session_id`, `data.session_ref`, `data.execution`,
`data.session_preserved`, and PTY output at `data.output.content` with
`data.output.kind == "pty"`. Inline exec output is bounded to 2 MiB:
`data.output.bytes` counts the full segment and `data.output.truncated` reports
omitted bytes. Use `session read` for the retained log instead of repeating the
command. Read returns text at `data.content` and up to 256 KiB of source bytes
per page. `--max-bytes N` lowers that budget (4..262144, default 262144); the
minimum holds one complete UTF-8 character. An incomplete trailing character
is held for the next page, so a page can be shorter than its budget.
Use `--tail N` to select the last N bytes of retained output, or `--since` set
to `data.next`, and continue while `data.more` is true. These offset flags are
mutually exclusive. With `--tail N --max-bytes M`, reading starts N bytes back
from the measured log end and returns at most M source bytes. `--tail 0`
returns an empty page at that end. Concurrent appends are left for later reads;
`data.size` describes the measured window, not a promise that no newer output exists.
Byte cursors must come from the response, not the rendered text length: terminal
sequences are removed from displayed content.

```bash
# Start with a bounded page from the recent log window.
rhost session read <host> debug --tail 8192 --max-bytes 4096 --json
# Continue using the preceding response's data.next while data.more is true.
rhost session read <host> debug --since <data.next> --max-bytes 4096 --json
```
`session attach` returns a non-retryable `USAGE_ERROR`; the v3
envelope has no honest terminal attachment representation.

`session create --cwd` records the *absolute* directory the account actually
entered, so `~` comes back as a real path and `create` and a later `list` agree.
Without `--cwd`, `initial_cwd` is omitted, and the session starts wherever the
login shell leaves it.

A create that reaches the host but does not return its full record reports the
candidate it reserved under `data.session_id`/`data.session_ref` and a
`data.creation_status` of `not_created` (the remote refused, or was seen to clean
up), `unknown` (a timeout, cancellation, disconnect or missing evidence), or
`created` (creation was observed but the rest of the response was not). Use
`session list` to confirm a candidate instead of retrying blind. A local
argument mistake or an id that cannot be generated still reports `data: null`.

