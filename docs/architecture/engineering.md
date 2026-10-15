# Engineering constraints

## Security and audit

OpenSSH owns authentication and host-key verification. rhost never persists
keys or passwords, weakens host-key policy, or installs remote dependencies.

Audit logging is local and fail-open. It records bounded operation metadata,
never environment maps, file contents or `session send --data` payloads.

## Repository boundaries

The tree is cut by capability: `domain`, `transport`, `remote` and `wire` are the
shared foundation, `cli` holds the flag/argv vocabulary, `dispatch` is the thin
composition root, and each capability (`exec`, `doctor`, `hosts`, `connection`,
`files`, `session`, `tunnel`, `audit`) owns its own grammar, use-cases and DTO.
`reference/` is read-only reference material and is not part of the product. The
archived Go v3.1.0 implementation lives in the read-only `rhost-go-old`
repository, not in this tree.

Durable state must remain outside the CLI process. A local daemon or remote
runtime is justified only after a measured requirement cannot be met by
OpenSSH, tmux and remote processes.

## Verification

Verification suites, commands, local gates, and release requirements are defined
authoritatively in [`AGENTS.md`](../../AGENTS.md#9-verify-before-reporting) and
[`docs/MAINTENANCE.md`](../MAINTENANCE.md#verification-gates).

Architectural acceptance criteria and contested-path frames live in
[`docs/ARCHITECTURE.md`](../ARCHITECTURE.md#observable-acceptance).

