# Safety, authority, and local audit

rhost has exactly the permissions of the configured OpenSSH identity. It is not
a sandbox, privilege broker, SSH replacement, or remote package manager.

## Credentials and command boundaries

Do not put secrets in command text, command files, `--env`, `session send`
arguments, or local audit metadata. Prefer a secret source already present on the
remote host or a trusted interactive input channel. Quoting ends at the local
shell boundary: inspect the complete local argv before destructive work.

Never weaken host-key verification or persist SSH keys/passwords. If a host key
changes, investigate before reconnecting. `RHOST_SSH_LOG_LEVEL=VERBOSE` changes
diagnostic verbosity only.

## Files

- Run sync or mirror with `--dry-run` before the same command with `--delete`.
- Delete is refused for broad destinations such as a whole home or top-level
  directory. There is no override flag.
- `fs write` may create a missing regular file. Replacement and every patch use
  compare-and-swap with a freshly observed SHA-256.
- Missing parents are created only with `--parents`; writes do not traverse a
  symlink and replacement is same-directory and atomic.
- Foreground transfer interruption can leave partial effects. Inspect both ends
  before retrying.

## Sessions and tunnels

Session writers share a remote lock. Do not bypass `SESSION_BUSY` or
`SESSION_UNHEALTHY` by typing a second command into an owned pane. Recover sends
an interrupt; it never guesses an exit command for a REPL.

Tunnels bind loopback by default. `--allow-exposure` deliberately permits other
machines to reach a listener, so confirm the bind side and network exposure.
Closing a tunnel affects only its dedicated OpenSSH master and canonical ID.

## Privilege and dependencies

rhost never installs a missing remote dependency. Establish the intended user or
privilege context before changing packages or login behavior, preserve an
independent recovery connection, and verify through a new SSH connection. Never
place a sudo password in arguments, files, audit data, or environment flags.

## State and audit

Remote v4 state resolves as
`${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost/v4}`. It holds session records and
short-lived exec pid evidence; it is separate from v3 state. Local v4 state is
under `${RHOST_STATE_DIR:-<platform-state-root>}/v4`, including `audit.jsonl` and
tunnel records. Do not put either state root in a repository.

The audit trail is bounded and fail-open. It records operation metadata and a
bounded command summary, never environment maps, file contents, or send payloads.
That omission is not a secrecy guarantee because process arguments remain
observable. `RHOST_AUDIT=0` disables new audit writes.
