# Rust ownership boundary

Run `make rust-check` after Rust changes. `make structure` enforces the shape:
a file that outgrows the reading budget fails, and so does a first-party
`crate::`/`rhost::` dependency the policy does not allow or one that closes a
cycle. The one place the permitted edges are written down is the `ALLOWED` map
in `scripts/check-structure.sh`; read it there (`./scripts/check-structure.sh -v`
prints it beside the graph the source actually produced). A module added here
must gain a policy entry there.

## Module map

| Path | Owns |
| --- | --- |
| `domain/` | values, transitions, completion evidence, CAS intents |
| `output/` | schema-v2 DTOs and one-document delivery |
| `fileops/` | path/argv rules, the local `scp`/`rsync` runner, the remote helper program |
| `transport/` | the OpenSSH child, the wrapper protocol, the process loop |
| `tunnel/` | one forward per dedicated OpenSSH master: local records, requests, the master itself |
| `session/` | the remote tmux helper programs and their line protocol |
| `audit.rs` | the local JSON Lines trail and its reader |
| `app/` | use-cases: exec, doctor, files (transfer, sync, edit, batch) |
| `cli/` | grammar, execution, console, rendering, usage prose |
| `main.rs` | signal handlers, parse, run, deliver |
| `tests/acceptance/` | one hermetic black-box test crate: `support` harness, `exec`, `files`, `edit`, `tunnel`, `session`, `audit`, `transport` |
| `tests/live_exec.rs`, `tests/live/` | feature-gated native live acceptance, split by capability and run serially against explicit `RHOST_BIN`/`RHOST_TEST_HOST` |

- `domain/`: validated identities, completion evidence, execution/session states,
  and CAS intents. No serde, crate-level dependencies, I/O, environment, process,
  thread or network access. Standalone rustc compilation and
  `pure_domain_boundary` enforce this boundary.
- `output/`: schema-v2 DTOs and one-document delivery. One module per capability
  (`exec`, `doctor`, `files`, `session`, `tunnel`); `mod.rs` owns the envelope and
  the shared error payload. Domain never derives serde.
- `cli/`: `mod.rs` is the surface (types plus dispatch); `grammar`/`commands` parse
  the non-file commands. The private `grammar.rs` facade routes whole invocations;
  `grammar/flags` owns flag tables and help metadata, and `grammar/scan` owns argv
  scanning, parsed values, command locating, JSON detection and durations.
  `run`/`tunnel` execute commands, `console` delivers,
  `render`/`usage` write prose. `files.rs` and `session.rs` are surfaces too:
  `files/parse` and `session/parse` turn argv into an operation, `files/run` and
  `session/run` execute one, and `files/input` reads a local body before anything
  is sent.
- `app/files/`: `transfer` moves one file, `sync` moves a tree, `edit` changes a
  file in place, `batch` sequences transfers, `remote` holds the one path out.
- `app/exec.rs`: the two entry points (`execute_stream`, `execute_captured`) over
  `outcome` (what a finished run means, and where the invocation token is spent),
  `capture` (bounded output) and `cleanup` (the remote stop attempt). `app/session`
  is cut the same way: `create`, `exec`, `io`, `lifecycle`, `errors`, `shared`.
- `fileops/`: `args` judges one copy's operands and builds scp's argv, `sync` does
  the same for rsync, `changes` reads rsync's itemized plan, and `mod.rs` owns the
  refusal type and re-exports.
- `transport/`: `mod.rs` is the only surface. `openssh`, `process` and
  `protocol` are private submodules, so a neighbor names the capability
  (`crate::transport::Client`) rather than an internal split; changing the child
  loop or the wrapper script does not mean finding every deep import.
  `transport/process.rs` splits into `capture` (bounded copies and sinks) and
  `run` (the child and its loop); `transport/protocol/stream.rs` owns the
  incremental marker-filtering reader.
- `fileops/`: `mod.rs` is the surface; `runner` and `remote` are private for the
  same reason.
- `tunnel/`: `record` owns the id and the on-disk record, `master` owns the
  dedicated OpenSSH process, and `mod.rs` owns requests. A tunnel's socket is its
  own on purpose: closing one forward must not drop the shared connection.
- `session/`: `scripts` holds the remote helper programs — one file per operation
  (`create`, `exec`, `io`, `lifecycle`) over `shared` — `protocol` parses what
  they print, and `mod.rs` owns the metadata and identity. The session itself is
  remote state: nothing here is remembered between invocations.
- `audit.rs` writes and reads the local trail; the CLI's `Timer` is the one place
  an operation is recorded, and it is fail-open by construction.
- `session attach` is the one schema operation that is refused: it needs a
  terminal, so it reports `USAGE_ERROR` under its real operation name rather than
  pretending to attach.
- Cargo forbids first-party unsafe and clippy rejects unwrap/expect. Do not add
  blanket lint allowances. Private domain modules expose only the facade in mod.rs.
- Review contract changes before implementation changes. Type errors do not
  authorize weakening a constructor or replacing distinct identities with String.
