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
adapter failures with `error.code`, completed commands with
`data.execution.exit_code`, and cleanup evidence with `data.cleanup.status`.
Unknown execution has no numeric exit code.

The detailed contracts and their rationale live in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Observable acceptance

- A foreground command reports the remote exit status and separate bounded
  stdout/stderr in one v2 JSON envelope; transport uncertainty does not invent
  successful completion. The local acceptance suite covers classification,
  and the native exec/transport suites verify it over SSH.
- A session created on a remote tmux server remains discoverable after the CLI
  exits. The native session suite verifies a second invocation against the same
  target; the CLI process has no durable session map.
- A file replacement requires the previously observed content hash, and an
  unusable target interpreter refuses before an edit. The local file acceptance
  suite covers both; the native file suite verifies actual remote operations.

The native suites are manual pre-release verification because CI has no SSH
target. Release CI checks the local contract and build artifacts separately.
