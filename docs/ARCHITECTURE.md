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

## Where code lives

The tree is cut by capability, not by layer. One directory owns one reason to
change, and a typical change touches that directory plus the shared surfaces it
names:

```text
domain  transport  remote  wire   shared foundation
cli                               flag/argv vocabulary, Sink, Failure
dispatch                          whole-argv grammar, help prose, routing
exec  doctor  hosts  connection   one vertical slice each
files  session  tunnel  audit     one capability each
```

`dispatch` is the composition root: it may depend on every capability, but it
only wires them together. Here, "the CLI" is an orchestration layer — it parses
argv and calls each capability's stable surface; it never reaches into a
capability's internals, and a capability never depends on `dispatch`. See
[src/AGENTS.md](../src/AGENTS.md) and the `ALLOWED` map in
`scripts/check-structure.sh` for the enforced edges.

## Detailed design

- [Runtime and foreground execution](architecture/runtime.md)
- [Persistent work](architecture/persistent-work.md)
- [Files and JSON contracts](architecture/files-and-json.md)
- [Engineering constraints](architecture/engineering.md)

[PROJECT_OVERVIEW.md](PROJECT_OVERVIEW.md) is the shorter product-level
description.

## Frames for contested paths

### OpenSSH adapter

- Default: invoke system OpenSSH and keep persistent work in OpenSSH, remote
  tmux, or remote files. Rival: embed an SSH stack or run a resident service.
- Breakpoint: a required operation that system tools cannot support with the
  documented guarantees, demonstrated by a reproducible case and reviewed as
  a product change.
- Invariant and check: direct runtime dependencies stay within the reviewed
  allowlist in `scripts/check-structure.sh`; `make structure` fails on additions.
  Live persistence still requires the manual remote suites before release.
- Oracle: the user's OpenSSH configuration and a real target, not a local
  substitute for authentication or host-key behavior.

### Remote filesystem helper

- Default: one embedded Python helper executed by the target's interpreter.
  Rival: a shipped native helper or generated shell program.
- Breakpoint: evidence of a supported target lacking a usable interpreter that
  cannot meet the file contract through existing tools. Record the case before
  changing the helper language; `fs batch` has its own compatibility decision.
- Invariant and check: the target must pass the helper's runtime preflight
  before any edit; the unusable-interpreter acceptance test requires
  `REMOTE_DEPENDENCY_MISSING`. `doctor` reports command presence only.
- Oracle: the target interpreter and filesystem features, not the developer's
  local `python3`.

### One-shot helper protocols

The exec wrapper, session helper, and filesystem helper are separate wire
protocols with one invocation-specific request and one bounded answer each.
Their shared invariant is that completion evidence belongs to the current
invocation. The transport, session, and filesystem acceptance tests exercise
that evidence and the corresponding refusal paths. A new helper protocol needs
its own Frame and evidence binding before it is added.
