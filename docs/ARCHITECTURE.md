# rhost architecture

## Intent

rhost lets a coding agent operate an SSH-reachable machine with the same command
model it uses locally:

```bash
rhost exec gpu --cwd '~/work' --command 'ls -a'
```

Text inside the command string is interpreted remotely. Flags outside it select
local rhost behavior. Stdout and stderr retain normal Unix pipeline behavior.

rhost is an adapter around existing Unix tools. It does not introduce a daemon,
replace OpenSSH authentication, or keep durable work inside the CLI process.

## System boundaries

```text
agent
  -> rhost CLI
     -> system OpenSSH / scp / rsync
        -> remote shell, tmux, schedulers and files
```

OpenSSH owns target resolution, authentication, host keys and reusable
connections. Remote tmux owns persistent sessions, and remote schedulers own
scheduled work. The filesystem owns file state. The CLI may exit without
invalidating any persistence promise.

## Product rules

- Direct remote execution is the primary interface.
- One exact shell program is supplied through `--command`; rhost never rebuilds
  shell syntax from multiple arguments.
- Every agent-visible behavior has a versioned JSON path.
- Timeouts preserve execution uncertainty and never turn it into a connectivity
  claim.
- Mutating file edits use compare-and-swap when replacing existing content.
- No package is installed automatically on the remote host.
- New abstraction requires a second implementation that needs it.

## Detailed design

- [Runtime and foreground execution](architecture/runtime.md)
- [Persistent work](architecture/persistent-work.md)
- [Files and JSON contracts](architecture/files-and-json.md)
- [Engineering constraints](architecture/engineering.md)

[PROJECT_OVERVIEW.md](PROJECT_OVERVIEW.md) is the shorter product-level
description.
