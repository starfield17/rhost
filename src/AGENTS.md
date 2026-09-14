# Rust ownership boundary

Run `make rust-check` after Rust changes. The map below is checked by
`make structure`: a file that outgrows the reading budget, or a dependency that
points the wrong way, fails the build rather than being a convention.

## Module map

| Path | Owns | May depend on |
| --- | --- | --- |
| `domain/` | values, transitions, completion evidence, CAS intents | nothing (no serde, no I/O, no env) |
| `output/` | schema-v2 DTOs and one-document delivery | `domain`, `app`, `host`, `transport`, `tunnel` |
| `fileops/` | path/argv rules, the local `scp`/`rsync` runner, the remote helper program | `domain`, `shell`, `stdio`, `transport` |
| `transport/` | the OpenSSH child, the wrapper protocol, the process loop | `config`, `domain`, `shell` |
| `tunnel/` | one forward per dedicated OpenSSH master: local records, requests, the master itself | `config`, `transport` |
| `session/` | the remote tmux helper programs and their line protocol | `base64`, `shell`, `transport` |
| `audit.rs` | the local JSON Lines trail and its reader | `clock` |
| `app/` | use-cases: exec, doctor, files (transfer, sync, edit, batch) | everything below it |
| `cli/` | grammar, execution, console, rendering, usage prose | `app`, `output`, `transport`, `fileops`, `tunnel`, `session` |
| `main.rs` | signal handlers, parse, run, deliver | `cli`, `signals`, `transport` |
| `tests/acceptance/` | one hermetic black-box test crate: `support` harness, `exec`, `files`, `edit`, `tunnel`, `session`, `audit`, `transport` | the built binary only |
| `tests/live_exec.rs`, `tests/live/` | feature-gated native live acceptance, split by capability and run serially against explicit `RHOST_BIN`/`RHOST_TEST_HOST` | the selected binary and a real host |

- `domain/`: validated identities, completion evidence, execution/session states,
  and CAS intents. No serde, crate-level dependencies, I/O, environment, process,
  thread or network access. Standalone rustc compilation and
  `pure_domain_boundary` enforce this boundary.
- `output/`: schema-v2 DTOs and one-document delivery. One module per capability
  (`exec`, `doctor`, `files`, `session`, `tunnel`); `mod.rs` owns the envelope and
  the shared error payload. Domain never derives serde.
- `cli/`: `mod.rs` is the surface (types plus dispatch); `grammar`/`commands` parse
  the non-file commands, `run`/`tunnel` execute them, `console` delivers,
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
- `transport/process.rs` splits into `capture` (bounded copies and sinks) and
  `run` (the child and its loop); `transport/protocol/stream.rs` owns the
  incremental marker-filtering reader.
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
