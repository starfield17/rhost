# Engineering constraints

## Security and audit

OpenSSH owns authentication and host-key verification. rhost never persists
keys or passwords, weakens host-key policy, or installs remote dependencies.

Audit logging is local and fail-open. It records bounded operation metadata,
never environment maps, file contents or `session send --data` payloads.

## Repository boundaries

Frozen Go packages under `archive/go-v3.1.0/internal/` own distinct capabilities: CLI parsing and
rendering, application policy, OpenSSH transport, remote session protocol,
and file operations. Remote scripts are generated or embedded by their owning
package. `reference/` is read-only reference material and is not part of the
product.

Durable state must remain outside the CLI process. A local daemon or remote
runtime is justified only after a measured requirement cannot be met by
OpenSSH, tmux and remote processes.

## Verification

```bash
make test-smoke
make check
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-smoke
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-session
RHOST_TEST_HOST=<user>@<host> RHOST_BIN=./target/debug/rhost make test-live-all
```

`make test-smoke` runs the complete local Rust black-box acceptance crate. Its
external-tool defaults refuse access, so it needs no SSH service or configured
host and cannot accidentally contact one. `make check` covers Rust formatting,
clippy, unit tests, independent DTO schema validation, domain boundaries,
portability, the structure budget, native-live compilation and coverage,
the release contract check and archive integrity.
The production Rust implementation covers every schema-v2 operation except
`session attach`, which the schema requires to be a usage error; see the current
[maintenance policy](../MAINTENANCE.md). Live suites are the gate that proves behavior across independent CLI processes on a real
remote Linux host. `test-live-smoke` is the frequent, bounded check of the main
exec, file and session workflows. The feature suites and `test-live-all`
retain the exhaustive failure, persistence and transport cases. The native full
suite is serial for deterministic ownership and cleanup, validates every JSON
answer against schema v2, and runs the exact binary named by `RHOST_BIN`.
The frozen Go harness is an optional historical comparison under
`test-legacy-live-*`; it is not required by the Rust release gate.

Tracked examples and verification claims must remain host-independent.
`scripts/check-portability.sh` enforces this.
