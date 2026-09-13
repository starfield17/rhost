# Persistent work

## Sessions

A session is a remote tmux session plus metadata under rhost's remote state
directory. Its cwd, environment and interactive program survive independent
rhost invocations.

`session exec` only writes into an idle managed shell. If a REPL or debugger
owns the pane it returns `SESSION_BUSY`; the agent drives that program with
`session send/read` or interrupts it with `session recover`.

`session send --data` is verbatim. It does not interpret backslash escapes.
Use `--data 'python3 -i' --enter` to paste text and press Enter in one
operation. Incremental reads use byte cursors and return UTF-8 content.

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

## Tunnels

Each tunnel is owned by a dedicated OpenSSH master and a local record needed to
rediscover and close it. An `alive` tunnel means the OpenSSH forward exists; it
does not claim that the application behind the destination is healthy.
