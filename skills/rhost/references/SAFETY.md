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

Direct `ssh` and `scp` on a minimal host use OpenSSH's authentication and host-key
policy, but do not add rhost's hash-guarded compare-and-swap, atomic replacement,
symlink refusal, or destructive-sync destination checks. Inspect the exact path
and current content before a mutation, and verify the result afterward. If the
task requires those guarantees, report that the required rhost operation is
unavailable rather than recreating it with an unguarded shell command.

## Sessions and tunnels

Session writers share a remote lock. Do not bypass `SESSION_BUSY` or
`SESSION_UNHEALTHY` by typing a second command into an owned pane. Recover sends
an interrupt; it never guesses an exit command for a REPL.

Tunnels bind loopback by default. `--allow-exposure` deliberately permits other
machines to reach a listener, so confirm the bind side and network exposure.
Closing a tunnel affects only its dedicated OpenSSH master and canonical ID.

## Privilege and dependencies

rhost never installs a missing remote dependency and never infers which package
supplies a tool: ask the host with `command -V <tool>` and the host's own package
manager (`dpkg -S`, `rpm -qf`, `brew --prefix`, ...), run through `exec`.

Do not rely on a `sudo` timestamp surviving across rhost calls. Each invocation is
its own submission, so a cache granted to an earlier call may have expired. When
`sudo` needs a password, use OpenSSH's own PTY — `ssh -t <host> 'sudo ...'` —
and never pass the password through a session `send`, a command argument, an
environment flag, or a temporary plaintext file.

Establish the intended user or privilege context before changing packages or
login behavior, preserve an independent recovery connection, and verify through a
new SSH connection. Packages and login-shell changes can move which binary a name
resolves to, so re-verify resolution in the ordinary-user context and again in the
real `sudo`/service context before trusting a long-running step.

After any account-state change, run `doctor --fresh` to see what the new login
resolves to, then `connection reset` to retire the old authentication snapshot so
the next call re-authenticates rather than reusing stale state.

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
