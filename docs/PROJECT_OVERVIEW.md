# rhost project overview

## Product boundary

A coding agent already knows how to run shell commands. rhost carries that
model across SSH while preserving the process contract the caller already
understands:

```bash
rhost exec gpu --cwd '~/work/project' --command 'git status --short'
```

The quoted `--command` value belongs to the remote shell. Other flags configure
rhost, and a pipeline after the invocation belongs to the local shell. That
typed boundary is the center of the product.

rhost is not a replacement SSH client, a remote development environment, or a
resident agent server. It is a command-line adapter between a coding agent and
an SSH-reachable machine.

## Ownership

The rhost process owns no durable remote work. OpenSSH owns target resolution,
authentication, host keys, ProxyJump, and connection reuse. Remote tmux owns
sessions. Remote schedulers own scheduled work. The filesystem owns file state.

Direct execution is therefore the default. Sessions, file operations, and
tunnels exist only where a foreground command cannot provide the required
guarantee cheaply. Work that must outlive the agent runtime belongs to a
scheduler already installed on the remote host and invoked explicitly through
direct execution; rhost does not become that scheduler.

Every agent-visible behavior also has a versioned JSON path. Callers classify
adapter failures with `error.code`, completed commands with `data.exit_code`,
and uncertain timeouts with `data.cleanup_confirmed`.

The detailed contracts and their rationale live in
[ARCHITECTURE.md](ARCHITECTURE.md).
