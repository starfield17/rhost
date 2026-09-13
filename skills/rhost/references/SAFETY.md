# Safety, authority, and the audit trail

## What rhost can do

Exactly what the current OS user can do through the configured SSH identity, and
no more. `rhost` does not store private keys or passwords, does not disable
host-key verification, does not rewrite OpenSSH configuration, and does not
elevate privileges. Authentication, host keys, `ProxyJump` and connection reuse
stay OpenSSH's job — rhost orchestrates the `ssh` binary, it does not replace it.

Two consequences worth stating plainly:

- A command run through rhost is a command run as your user on that host. There is
  no sandbox and no command firewall: the SSH account's permissions are the
  permissions.
- rhost never installs remote packages, not even to make a feature work. A
  missing dependency is reported, not fixed.

## Secrets

**Do not put a secret on a command line.** Command text is visible to `ps` on the
remote host and is written into the local audit log (truncated, but present). Use
the remote environment, a file that already holds the secret, or the mechanism
the tool itself provides.

```bash
# no: the token is in the audit log, the process table, and your shell history
rhost --host <host> -- 'curl -H "Authorization: Bearer <token>" https://api.example'

# yes: the value stays on the remote side
rhost --host <host> -- 'curl -H "Authorization: Bearer $GITHUB_TOKEN" https://api.example'
```

`session send --data` records the *key* or the fact that data was sent, never the
injected bytes. `fs` operations record paths, not contents. Environment variables
passed with `--env` are an interface for configuration, not for secrets: they
travel through the remote process environment and are visible there.

## Destructive file operations

- `fs sync` and `fs mirror` copy without pruning by default. With `--delete`,
  `sync` prunes entries present only in the remote destination and `mirror`
  prunes entries present only in the local destination. Run the same command
  with `--dry-run` first: `data.changes` is the applying run's plan, and
  `data.deletes` counts its pruning entries.
- `--delete` is refused (`SYNC_REJECTED`) when the destination is a top-level
  directory or an entire home. Sync into a subdirectory instead; there is no flag
  that makes pruning a whole home safe.
- A destination containing glob characters is refused, because the remote shell
  would expand it into something you did not name.
- `fs write` can create a missing file without a hash. Replacing a file and
  every `fs patch` require the hash of the file you last read. That is the
  compare-and-swap: a hash you did not just read is a guess about someone else's
  current contents, and it is refused rather than applied.
- `fs put` and `fs write` create missing parent directories only with an
  explicit `--parents`.
- Writes are atomic within the directory and are never applied through a symlink.
- Everything about `fs` is foreground: if this process dies, the copy stops. A
  transfer you need to survive a disconnect is a `job` running `rsync` or `scp`
  on the remote side.

## Tunnels

- Forwards bind to loopback unless `--allow-exposure` is given. That flag is a
  request to let other machines through: a reverse forward can put a remote
  service on your local network, and a local forward can put a local port on the
  remote one.
- A tunnel is a dedicated OpenSSH master that outlives the process that created
  it. Keep the canonical `data.tunnel_id` (`data.id` is an identical compatibility
  alias); `tunnel list` is the only way back to an id you did not keep. Nothing
  restarts a tunnel after a reboot.
- `alive` means the forward exists, not that a service answers behind it. rhost
  does not probe the far end for you.

## Host keys and connection health

Host-key verification is OpenSSH's, and rhost never weakens it. If a host key
changed, find out why before connecting again. `RHOST_SSH_LOG_LEVEL=VERBOSE`
raises OpenSSH's own log level for one command when a connection failure has no
diagnostic; it does not change any security setting.

## The local audit trail

`rhost` writes a local JSONL log of the remote operations it performs —
`~/.local/state/rhost/audit.jsonl` (or under `RHOST_STATE_DIR`) — and
`rhost audit --json` reads it back (most recent 20 by default, `--limit 0` for
all, `--host H` to filter). `RHOST_AUDIT=0` turns it off.

It records bounded operation metadata: no environment maps, no file contents, no
`session send` payloads, and a command summary truncated to one bounded line.
That is a bound, not a filter: a secret typed into a command line is still
recorded, which is why secrets do not belong there.

Auditing is fail-open: a write failure never blocks the remote operation. Direct
commands, file writes and transfers, every batch entry, session mutations, job
mutations, tunnel open/close, and `doctor` are recorded, including failures.
Read-only `fs read`, `hosts`, and `audit` are not. The file is local state; do not
place `RHOST_STATE_DIR` in a repository or upload the audit log to GitHub.
