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

## Jobs

A job is a detached remote process group plus remote metadata, output files and
an exit-code file. Starting, listing, reading and signalling jobs may happen
from different CLI processes.

Process identity is checked before signals are sent so a recycled pid is never
treated as the original job. A running or otherwise unresolved job reports
`exit_code:null`; a recorded final exit reports an integer.

Job logs are byte streams and therefore use explicit base64 encoding. Cursors
refer to decoded remote bytes.

## Tunnels

Each tunnel is owned by a dedicated OpenSSH master and a local record needed to
rediscover and close it. An `alive` tunnel means the OpenSSH forward exists; it
does not claim that the application behind the destination is healthy.
