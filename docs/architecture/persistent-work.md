# Persistent work

## Sessions

A session is a remote tmux session plus metadata under rhost's remote state
directory. Its cwd, environment and interactive program survive independent
rhost invocations.

`session exec` only writes into an idle managed shell. If a REPL or debugger
owns the pane it returns `SESSION_BUSY`; the agent drives that program with
`session send/read` or interrupts it with `session recover`. The session surface
is intentionally frozen at create/list/exec/send/read/attach/recover/close: it
owns terminal state, not windows, layouts, process management, or scheduling.
Schema v3 recognizes `attach` but refuses it with `USAGE_ERROR`; agents use
send/read rather than an unrepresentable attached terminal.

`session send --data` is verbatim. It does not interpret backslash escapes.
Use `--data 'python3 -i' --enter` to paste text and press Enter in one
operation. Incremental reads use byte cursors and return UTF-8 content.

`session exec` and `session recover` accept fractional timeout values. Their
remote helper uses a whole-second clock and rounds a fraction up to the next
second, so the remote budget never truncates below the requested duration.

The tmux helper is an internal line protocol. Successful `session exec` responses
carry one resolved `RHOST_ID`, exactly one `RHOST_TOKEN`, `RHOST_EXIT`, and base64-encoded `RHOST_OUTPUT`;
`session read` responses carry exactly one non-negative `RHOST_FROM`, `RHOST_NEXT`,
and `RHOST_SIZE` followed by base64 content. Missing, duplicate, malformed, or
inconsistent fields are protocol failures and become `SESSION_UNHEALTHY`; they
are never treated as an empty successful result.

## Recoverable noninteractive work

Use native Runs for builds/tests and work requiring separate streams and durable
completion. A caller-owned expiring key and namespace generation allow later CLI
processes to rediscover the launch. Wait/cancellation stop observation; remote
deadlines and signals control work explicitly. Monitor death and missing results
preserve uncertainty rather than authorizing a second launch or saved-PID kill.
See [native Run semantics](agent-interface-v5.md).

Foreground `exec` retains its existing streaming, timeout and cleanup semantics.
Interactive REPL/debugger state remains in tmux sessions. Scheduled/restarted
services belong to an existing remote scheduler selected by the caller.
Unmanaged background processes remain the caller's responsibility; their
submission or disappearance is not completion evidence.

## Tunnels

Each tunnel is owned by a dedicated OpenSSH master and a local record needed to
rediscover and close it. An `alive` tunnel means the OpenSSH forward exists; it
does not claim that the application behind the destination is healthy.
