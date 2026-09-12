# rhost — Project Overview

## Intent

`rhost` lets a coding agent run an existing shell command on a remote machine as
naturally as it runs one locally:

```text
local command intent
        │
        ▼
remote execution context (host, cwd, env)
        │
        ▼
OpenSSH → remote command
        │
        ▼
stdin / stdout / stderr / exit status returned locally
```

The product abstraction is a command executed in a remote context. A remote host
is not a resource model that rhost must reproduce, and a command that already
exists on the remote system does not need a dedicated rhost wrapper.

## Product rule

Add a dedicated operation only when it supplies a guarantee that an ordinary
remote command cannot cheaply and reliably compose. Current examples are durable
jobs, persistent interactive sessions, cross-machine file transfer,
hash-protected editing, and persistent tunnels.

Host monitoring, remote search, and multi-host fan-out are composed with ordinary
remote commands or the caller's own loop. They are not separate product concepts.

## Execution semantics

Direct foreground execution is the default:

```bash
rhost --host gpu --cwd '~/work/project' -- 'pytest -q'
```

- The text after `--` is one shell program, not an argv array reconstructed by
  joining words.
- Human mode inherits stdin, forwards both output streams, and waits without a
  default deadline.
- JSON mode captures bounded output into one versioned envelope.
- A timeout or local signal attempts to terminate the remote process group and
  reports whether cleanup was confirmed.
- Commands are never automatically retried or silently converted into jobs.

Sessions and jobs remain explicit because they promise different ownership:

| need | owner |
|---|---|
| one foreground command | fresh SSH channel and remote process group |
| interactive state across calls | remote tmux session and remote log |
| work surviving disconnect | detached remote process and remote metadata |

## Architecture constraints

The CLI is disposable. Anything promised to survive it must be owned elsewhere:

- OpenSSH ControlMaster owns reusable connections;
- remote tmux owns sessions;
- remote process groups and state files own jobs;
- OpenSSH owns authentication, host-key policy, and SSH config resolution.

There is no hidden local server, remote daemon, credential store, automatic
package installation, project-specific command family, or in-memory-only durable
state.

Human output and JSON use the same application semantics. The JSON envelope is a
versioned contract, and agents branch on `error.code` rather than English text.

## Reference project

`reffer/portal-mcp-server` is useful for execution boundaries, cancellation,
structured results, file safety, and background-job mechanics. Its MCP-first
lifecycle, in-process SSH ownership, credential system, and broad tool catalog do
not define rhost's product shape.

The inclusion test borrowed from the reference is the useful part: retain a
specialized operation only when it supplies a guarantee bash cannot cheaply
synthesize.

## Success criteria

The ordinary loop should require no remote-specific reasoning beyond choosing the
target and remote directory:

```bash
rhost --host gpu --cwd '~/work/project' -- 'git status --porcelain'
rhost --host gpu --cwd '~/work/project' -- 'go test ./...'
rhost --host gpu --cwd '~/work/project' -- 'rg TODO'
```

The agent receives the same command output and status it would inspect locally.
It reaches for `job`, `session`, `fs`, or `tunnel` only when the task actually
requires their extra lifetime or safety guarantee.
