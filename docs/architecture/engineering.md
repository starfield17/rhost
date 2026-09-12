# Engineering constraints

## Security and audit

OpenSSH owns authentication and host-key verification. rhost never persists
keys or passwords, weakens host-key policy, or installs remote dependencies.

Audit logging is local and fail-open. It records bounded operation metadata,
never environment maps, file contents or `session send --data` payloads.

## Repository boundaries

Go packages under `internal/` own distinct capabilities: CLI parsing and
rendering, application policy, OpenSSH transport, remote session/job protocol,
and file operations. Remote scripts are generated or embedded by their owning
package. `reffer/` is read-only reference material and is not part of the
product.

Durable state must remain outside the CLI process. A local daemon or remote
runtime is justified only after a measured requirement cannot be met by
OpenSSH, tmux and remote processes.

## Verification

```bash
make check
RHOST_TEST_HOST=<user>@<host> make test-live-smoke
RHOST_TEST_HOST=<user>@<host> make test-live
RHOST_TEST_HOST=<user>@<host> make test-live-session
RHOST_TEST_HOST=<user>@<host> make test-live-all
```

`make check` covers formatting, vet, unit tests, portability and the release
contract. Live suites prove behavior across independent CLI processes on a real
remote Linux host. `test-live-smoke` is the frequent, bounded check of the main
exec, file, session and job workflows. The feature suites and `test-live-all`
retain the exhaustive failure, persistence and transport cases. Independent
live tests run with at most three tests in parallel. Each active test leases one
of three persistent OpenSSH ControlMasters, so its CLI processes reuse a
connection without competing with another test for channels on that connection.
Intentionally isolated ControlPath cases remain serial. The harness prints CLI
call count and timing totals so regressions are visible. Live targets disable
Go's result cache so every invocation reaches the named host.

Tracked examples and verification claims must remain host-independent.
`scripts/check-portability.sh` enforces this.
