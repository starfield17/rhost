# Rust ownership boundary

Run `make rust-check` after Rust changes. `make structure` enforces the shape:
a file that outgrows the reading budget fails, and so does a first-party
`crate::`/`rhost::` dependency the policy does not allow or one that closes a
cycle. The one place the permitted edges are written down is the `ALLOWED` map
in `scripts/check-structure.sh`; read it there (`./scripts/check-structure.sh -v`
prints it beside the graph the source actually produced). A module added here
must gain a policy entry there.

## Module map

The tree is cut by capability: one directory owns one reason to change, and a
change to a capability should need only that directory plus the shared surfaces
it names. `app/`, `output/` and `fileops/` no longer exist; their work lives
with the capability that owns it.

| Path | Owns |
| --- | --- |
| `domain/` | values, transitions, completion evidence, CAS intents (pure) |
| `transport/` | the OpenSSH child, the wrapper protocol, the process loop |
| `remote/` | the one submit-a-program path and the `Error` taxonomy |
| `wire/` | the schema-v2 envelope, delivery, and the shared execution DTO |
| `cli/` | flag tables, the argv scanner, `Sink`/`Failure` — capability-agnostic |
| `dispatch/` | the whole-argv grammar, the subcommand inventory, help prose, routing |
| `audit/` | the local JSON Lines trail, its command, and the shared `Timer` |
| `exec/` | `rhost exec`: grammar, run, human status |
| `doctor/` | the capability probe and its envelope |
| `hosts/` | local OpenSSH client-config discovery and its envelope |
| `connection/` | control-master status/reset; the shared `ConnectionDto` |
| `files/` | `fs`: grammar, use-cases, tool/helper backend, envelope (old `fileops`) |
| `session/` | remote tmux sessions: grammar, use-cases, helper protocol/scripts, envelope |
| `tunnel/` | one forward per dedicated OpenSSH master: records, requests, the master |
| `main.rs` | signal handlers, parse, run, deliver |
| `tests/acceptance/` | one hermetic black-box test crate, split by capability |
| `tests/live_exec.rs`, `tests/live/` | feature-gated native live acceptance, split by capability |

- **Shared foundation** (`domain`, `transport`, `remote`, `wire`, `cli`) is the
  only thing more than one capability may depend on. A capability never depends
  on a sibling capability; the one declared exception is `doctor -> connection`,
  which reuses the shared master DTO.
- `dispatch` may depend on every capability because wiring is its whole job; it
  holds no capability behavior. `main` also uses `transport` and `signals` for
  process setup and shutdown; the `ALLOWED` map is authoritative.
- `domain/`: validated identities, completion evidence, execution/session states
  and CAS intents. No serde, crate-level dependencies, I/O, environment, process,
  thread or network access. Standalone rustc compilation and `pure_domain_boundary`
  enforce this boundary.
- `cli/`: the flag vocabulary and argv scanner every capability parses with, the
  stdout `Sink`, and `Failure` (the one mapping from an `error.code` to a process
  status). It knows nothing about what any command means. A capability parser
  returns `ParsedCommand<X>` — run, help, or a usage refusal — and `dispatch`
  lifts it into a routed `Command`.
- `dispatch/`: `route` is the only whole-argv parser; `commands` holds the
  root-owned `version`; `usage` holds the help prose; `run` is the one match that
  fans out to capabilities. `parse_invocation` is the only entry point the binary
  uses.
- `wire/`: the `Envelope`, its one-document delivery, `error`/`failure`, and the
  `exec` DTO (`execution.rs`) that `doctor` reuses on a failed probe. Capability
  DTOs live with their capability.
- `exec/`, `doctor/`, `hosts/`, `connection/`: vertical slices — grammar, run and
  envelope in one directory.
- `files/`: `command` (grammar), `surface` (the parsed `Fs`), `ops` (transfer,
  sync, edit, batch, the one path out), `backend` (path/argv rules, the local
  `scp`/`rsync` runner, the remote helper program), `dto`, `render`. `transport`
  stays a neighbor, not a member.
- `session/`: `command`/`run`/`dto`/`render` plus `ops` (create, exec, io,
  lifecycle, errors, shared) and `remote` (the helper `scripts` and their
  `protocol`). The session itself is remote state; nothing here is remembered
  between invocations. `session attach` is the one schema operation refused with
  `USAGE_ERROR`.
- `tunnel/`: `record` owns the id and the on-disk record, `master` owns the
  dedicated OpenSSH process, `command`/`run`/`dto`/`render` are the surface.
- `transport/`: `mod.rs` is the only surface; `openssh`, `process` and `protocol`
  are private, so a neighbor names the capability (`crate::transport::Client`).
- Cargo forbids first-party unsafe and clippy rejects unwrap/expect. Do not add
  blanket lint allowances. Private domain modules expose only the facade in mod.rs.
- Review contract changes before implementation changes. Type errors do not
  authorize weakening a constructor or replacing distinct identities with String.
