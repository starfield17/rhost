# rhost architecture

## Intent

rhost lets a coding agent operate an SSH-reachable machine with the same command
model it uses locally:

```bash
rhost exec gpu --cwd '~/work' --command 'ls -a'
```

Text inside the command string is interpreted remotely. Flags outside it select
local rhost behavior. Stdout and stderr retain normal Unix pipeline behavior.

rhost combines existing Unix tools with a narrow native agent interface. It does not introduce a daemon,
replace OpenSSH authentication, or keep durable work inside the CLI process.

## System boundaries

```text
agent
  -> rhost CLI / stdio MCP
     -> system OpenSSH / scp / rsync
        -> native C99 helper / per-Run monitor, shell, tmux and files
```

OpenSSH owns target resolution, authentication, host keys and reusable
connections. Remote tmux owns persistent sessions, per-Run monitors own
recoverable execution, and existing schedulers own scheduled/restarted work.
The filesystem owns file and request state. The CLI may exit without
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

Small root modules (`config`, `shell`, `base64`, `clock`, `random`, `stdio`,
`signals`, `host`) own concrete shared primitives. `audit` also provides the
operation trail and shared timer to capability runners.

`dispatch` is the composition root: it may depend on every capability, but it
only wires them together. Here, "the CLI" is an orchestration layer — it parses
argv and calls each capability's stable surface; it never reaches into a
capability's internals, and a capability never depends on `dispatch`. See
[src/AGENTS.md](../src/AGENTS.md) and the `ALLOWED` map in
`scripts/check-structure.py` for the enforced edges. Run `make structure` to
check them, or `./scripts/check-structure.sh -v` to inspect policy and actual edges.

## Detailed design

- [Runtime and foreground execution](architecture/runtime.md)
- [Persistent work](architecture/persistent-work.md)
- [Files and JSON contracts](architecture/files-and-json.md)
- [Engineering constraints](architecture/engineering.md)

### Native agent interface

[Native agent interface](architecture/agent-interface-v5.md) describes the C99
helper, recoverable Runs, structured request/MCP delivery and exact-artifact
verification. [Targeted file changes](architecture/file-changes-v5.md) describes
recoverable named edits without workspace or Git ownership.

For getting started, use the [README](../README.md). The
[contract ledger](CONTRACT.md) owns required behavior and executable evidence;
the [schema](../schemas/result-v3.schema.json) owns JSON shape.
[Maintenance policy](MAINTENANCE.md) owns compatibility, change risk and release
requirements; the migration record is frozen history.

## Observable acceptance

- Foreground execution reports a known remote exit status and separate bounded
  stdout/stderr in one v3 envelope. Missing completion remains unknown. Local
  acceptance covers classification; the native exec/transport suites exercise SSH.
- A tmux session remains discoverable by a later CLI invocation. The native
  session suite exercises this persistence; no local session map is authoritative.
- File replacement requires the previously read hash. Local helper and CLI
  acceptance cover bounded reads, whole-file hashes, conflicts and dependency
  refusal; the native file suite exercises remote filesystem operations.

Native suites provide manual pre-release evidence. CI checks the local contract
and release artifacts, including disposable real-sshd helper tests. Real-host
manual verification is separately bound through an operator attestation.

## Frames for contested paths

### OpenSSH adapter

- Default: invoke system OpenSSH and keep persistent work in OpenSSH, remote
  tmux, or remote files. Rival: embed an SSH stack or run a resident service.
- Breakpoint: a required operation that system tools cannot support with the
  documented guarantees, demonstrated by a reproducible case and reviewed as
  a product change.
- Invariant and check: direct runtime dependencies stay within the reviewed
  allowlist in `scripts/check-structure.py`; `make structure` fails on additions.
  Live persistence still requires the manual remote suites before release.
- Oracle: the user's OpenSSH configuration and a real target, not a local
  substitute for authentication or host-key behavior.

### Native helper

- Selected path: an independent C99 executable in a private content-addressed
  cache. Remote Python is no longer a file-editing prerequisite.
- Boundary: no global daemon, package installation, workspace creation or Git
  management. Fixed source/configuration and selected artifact hashes identify
  the helper; the remote account and artifact builder remain trusted.
- Checks: no-follow/ownership constraints, exact candidate handshake, unchanged
  file conformance, shipping-byte native Linux/SSH acceptance and local sanitizers.
- Oracle: actual remote process/filesystem behavior, not compiler success alone.

### One-shot helper protocols

The exec wrapper, session helper, and filesystem helper are separate wire
protocols with one invocation-specific request and one bounded answer each.
Their shared invariant is that completion evidence belongs to the current
invocation. The transport, session, and filesystem acceptance tests exercise
that evidence and the corresponding refusal paths. A new helper protocol needs
its own Frame and evidence binding before it is added.
