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
Schema v2 recognizes `attach` but refuses it with `USAGE_ERROR`; agents use
send/read rather than an unrepresentable attached terminal.

`session send --data` is verbatim. It does not interpret backslash escapes.
Use `--data 'python3 -i' --enter` to paste text and press Enter in one
operation. Incremental reads use byte cursors and return UTF-8 content.

The tmux helper is an internal line protocol. Successful `session exec` responses
carry one resolved `RHOST_ID`, exactly one `RHOST_TOKEN`, `RHOST_EXIT`, and base64-encoded `RHOST_OUTPUT`;
`session read` responses carry exactly one non-negative `RHOST_FROM`, `RHOST_NEXT`,
and `RHOST_SIZE` followed by base64 content. Missing, duplicate, malformed, or
inconsistent fields are protocol failures and become `SESSION_UNHEALTHY`; they
are never treated as an empty successful result.

## Long-running work

An ordinary `rhost exec` remains foreground from rhost's perspective. An agent
runtime may put that local invocation in the background when it only needs to
continue other work concurrently; rhost still owns streaming and the eventual
process result.

Work that must survive the agent runtime belongs to an existing remote
scheduler. The caller invokes that scheduler explicitly through `rhost exec`
and uses its native status, log, cancellation, and retention interfaces. rhost
does not install, select, or emulate a scheduler, and sessions are not a generic
job-management substitute.

If a caller deliberately starts unmanaged background work, the entire background
group must redirect stdin, stdout and stderr away from SSH and record its own
log, process identity and completion status. A vanished process without a
completion record is unknown; matching a process name is not proof of success.
Silence or a harness interruption does not prove that remote work is stuck.

## Tunnels

Each tunnel is owned by a dedicated OpenSSH master and a local record needed to
rediscover and close it. An `alive` tunnel means the OpenSSH forward exists; it
does not claim that the application behind the destination is healthy.
