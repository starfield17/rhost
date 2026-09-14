# Rust ownership boundary

Run `make rust-check` after Rust changes.

- `domain/`: validated identities, completion evidence, execution/session states,
  and CAS intents. No serde, crate-level dependencies, I/O, environment, process,
  thread or network access. Standalone rustc compilation and
  `pure_domain_boundary` enforce this boundary.
- `output/`: schema-v2 DTOs and one-document delivery. Domain never derives serde.
- `main.rs`: only version/usage is implemented. Remote command support is pending.
- Cargo forbids first-party unsafe and clippy rejects unwrap/expect. Do not add
  blanket lint allowances. Private domain modules expose only the facade in mod.rs.
- Review contract changes before implementation changes. Type errors do not
  authorize weakening a constructor or replacing distinct identities with String.
