# rhost — Rust v4 migration

rhost is being reimplemented in Rust from explicit behavioral contracts.
The current 4.0.0-alpha.1 source is a **domain and output skeleton**:
version/help work; SSH exec, files, tunnels and sessions are not implemented.

The complete Go v3.1.0 source, version metadata, tests, installer and release
configuration are frozen in [archive/go-v3.1.0](archive/go-v3.1.0).
Use its [README](archive/go-v3.1.0/README.md) for the legacy operational CLI.
The owner reports the Go baseline passed real SSH verification.

Build and check the active Rust tree:

```sh
make build
make check
./target/debug/rhost version --json
```

Cargo.toml owns the Rust binary version. Schema version is independent;
the Rust target is schema v2. Go's frozen VERSION remains 3.1.0.

Start with [contracts](docs/CONTRACT.md) and the
[migration status and next steps](docs/RUST_MIGRATION.md).
The [archive](archive/README.md) is read-only. New code and tests use Rust.
The four-platform workflow currently rehearses artifacts manually; it does not
publish a release or establish SSH parity.
