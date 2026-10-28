# Rust ownership boundary

Run `make rust-check` after Rust changes. `make structure` enforces the shape:
a file that outgrows the reading budget fails, and so does a first-party
`crate::`/`rhost::` dependency the policy does not allow or one that closes a
cycle. The one place the permitted edges are written down is the `ALLOWED` map
in `scripts/check-structure.py`; read it there (`./scripts/check-structure.sh -v`
prints it beside the graph the source actually produced). A module added here
must gain a policy entry there.

## Module map

The tree is cut by capability: one directory owns one reason to change, and a
change to a capability should need only that directory plus the shared surfaces
it names.

| Path | Owns |
| --- | --- |
| `domain/` | values, transitions, completion evidence, CAS intents (pure) |
| `helper/` | native helper sources, RH01 codec, preparation and raw SSH invocation |
| `request/` | closed outer JSON requests and delivery-neutral capability composition |
| `mcp/` | official SDK stdio adapter, scoped observation cancellation |
| `runs/` | recoverable Run requests, CLI views, validation and observation DTOs |
| `transport/` | the OpenSSH child, the wrapper protocol, the process loop |
| `remote/` | the one submit-a-program path and the `Error` taxonomy |
| `wire/` | the schema-v3 envelope, delivery, and the shared execution DTO |
| `cli/` | flag tables, the argv scanner, `Sink`/`Failure` — capability-agnostic |
| `dispatch/` | the whole-argv grammar, the subcommand inventory, help prose, routing |
| `audit/` | the local JSON Lines trail, its command, and the shared `Timer` |
| `exec/` | `rhost exec`: grammar, run, human status |
| `doctor/` | the capability probe and its envelope |
| `hosts/` | local OpenSSH client-config discovery and its envelope |
| `connection/` | control-master status/reset; the shared `ConnectionDto` |
| `files/` | `fs`: grammar, use-cases, tool/helper backend, envelope |
| `session/` | remote tmux sessions: grammar, use-cases, helper protocol/scripts, envelope |
| `tunnel/` | one forward per dedicated OpenSSH master: records, requests, the master |
| `main.rs` | signal handlers, parse, run, deliver |
| root primitives | `config`, `shell`, `base64`, `clock`, `random`, `stdio`, `signals`, `host` |
| `../tests/acceptance/` | one hermetic black-box test crate, split by capability |
| `../tests/live_exec.rs`, `../tests/live/` | feature-gated native live acceptance, split by capability |

- Capabilities use the shared foundation and the concrete root primitives
  allowed by `ALLOWED`. Capability runners may also depend on `audit` for the
  trail and timer. The other permitted capability edge is `doctor -> connection`,
  which reuses the shared master DTO. No other sibling edges are permitted.
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
  `scp`/`rsync` runner, the typed remote helper protocol), `dto`, `render`. `transport`
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

## Locate a change

Paths below are relative to the repository root. Read the owner and relevant
contract rows, then expand only for an unresolved dependency or failure.

| Task | Entry / implementation | Contract / verification |
| --- | --- | --- |
| Flags, routing, help | `src/dispatch/`, capability `command.rs`, `src/cli/flags.rs` | `WIRE-*`; `cargo test --locked --test cli` |
| Foreground execution / SSH | `src/exec/`, `src/transport/`, `src/remote/` | `EXEC-*`, `SSH-*`; acceptance `exec::`, `transport::` |
| File input / editing / copies | `src/files/command.rs`, `input.rs`, `ops/`, `backend/` | `FS-*`; acceptance `edit::`, `remote_fs::`, `files::`, `examples::` |
| Persistent terminals | `src/session/command.rs`, `ops/`, `remote/` | `SESSION-*`; acceptance `session::`; manual `make test-live-session` |
| Tunnels / audit / discovery | matching capability `command` / `run` and DTO | `TUNNEL-*`, `AUDIT-*`, `SSH-*`; matching acceptance module |
| JSON / contract evidence | capability `dto`, `src/wire/`, `docs/CONTRACT.md` | active schema; `cargo test --locked --test contracts --test contract_evidence` |

For an acceptance module, run `cargo test --locked --test acceptance '<module>::'`.
`make check` remains the complete local gate; remote suites require explicit
`RHOST_TEST_HOST` and `RHOST_BIN` as documented in the root instructions.
