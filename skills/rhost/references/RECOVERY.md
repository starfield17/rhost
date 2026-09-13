# Recovering from a failed rhost call

Read `ok`, `error.code`, `error.retryable` and the operation's `data` from the
JSON envelope. The message is for the human reading the transcript; the code is
what to branch on. Several answers preserve uncertain remote state rather than
pretending a failed local call proves the remote side stopped.

## Codes, and what each asks for

Connection and authentication:

- `SSH_UNREACHABLE` — retryable. If the message says OpenSSH gave no diagnostic,
  its reason was suppressed at the default log level: re-run once with
  `RHOST_SSH_LOG_LEVEL=VERBOSE` to see it.
- `SSH_AUTH_FAILED` — a key or agent problem on this machine. Fix it here; blind
  retries cannot.
- `HOST_KEY_FAILED` — the host key changed. Investigate (rebuild? wrong host?),
  never disable checking.
- `HOST_UNKNOWN` — the target is not something OpenSSH accepts.

The remote is missing something:

- `REMOTE_DEPENDENCY_MISSING` — `rhost doctor <host>` names what is missing and
  what still works without it. Sessions need `tmux` and `flock`; `fs sync` needs
  `rsync`; `fs read/write/patch` needs `python3`.
  rhost never installs anything for you.
- `UNSUPPORTED_REMOTE_OS` — the probe could not establish a supported platform.

Time:

- `REMOTE_COMMAND_TIMEOUT` — the deadline passed. For `exec`, check
  `data.cleanup_confirmed`: when true the remote process group was observed and
  killed and `error.retryable` may be true; when false the command may still be
  running and `error.retryable` is false. Check state before another side
  effect. For a session, check `data.session_preserved`: false means something
  still holds the pane (`session read` to see what, `session recover` to
  interrupt it). If work
  must survive the agent runtime, submit it explicitly to a scheduler already
  installed on the remote host through `rhost exec`.
- `REMOTE_COMMAND_CANCELLED` — the local rhost received SIGINT or SIGTERM.
  `data.cancel_signal` names that signal when `data.cancelled` is true; the field
  is absent otherwise. Check it and `data.cleanup_confirmed` before retrying a
  side effect.
- `OUTPUT_WRITE_FAILED` — the local stdout/stderr consumer closed or failed.
  Do not infer that the remote operation failed or retry a side effect. For a
  streamed exec, remote cleanup was attempted; inspect `data.cleanup_confirmed`
  when it is available, otherwise check remote state separately. If writing the
  final JSON document itself failed, only stderr and process status 255 may remain.

Sessions:

- `SESSION_BUSY` — retryable, and *not* damage. A program owns the pane, so the
  managed command was not run at all. Drive the program with `session send` and
  `session read`, or interrupt it with `session recover`, then exec again.
- `SESSION_UNHEALTHY` — the session did not answer (the pane is wedged by a
  `sudo` prompt or an interactive program), another writer holds the lock, or the
  helper could not read the pane. `session read` first; `session recover`
  interrupts; Retryable where the lock is the cause.
- `SESSION_NOT_FOUND` — the session is gone: closed, or ended by `exit` inside
  it. A recovery never recreates a session, because doing so silently discards
  the state that was being kept. Create a new one if that is acceptable.

Files:

- `FILE_NOT_FOUND` — the remote path does not exist. Inspect it with a direct
  remote command such as `test`, `ls`, or `find` instead of guessing.
- `HASH_REQUIRED` — the target already exists but no comparison hash was
  supplied. Read it with `fs read`, merge if needed, and use that fresh SHA-256;
  do not bypass or guess the precondition.
- `FILE_CONFLICT` — the hash you supplied no longer describes the file, or the
  file changed during the operation. Read it again and merge; never replay an old
  hash.
- `INVALID_TARGET` — rhost will not write through a symlink, into a directory, or
  onto anything that is not a regular file. Inspect the path; do not retry.
- `INVALID_PATCH` — the edit ranges overlap, are not 1-based inclusive, or run
  past the end. The file was not touched.
- `INVALID_TEXT` / `FILE_TOO_LARGE` — the bytes are not UTF-8, or exceed the 8 MiB
  editing limit. Use `fs put`, `fs get` or `fs sync` for binary and large
  content.
- `TRANSFER_FAILED` — the transfer tool said no; its own first complaint is in
  `message`. `SYNC_REJECTED` is the subset that rhost refused before running
  anything: a destination that is a glob, a whole home, or a top-level directory
  with `--delete`.
- `fs batch` keeps per-entry results in `data.items[]` even when the run fails
  overall: the aggregate exit status does not name which entry failed.

Tunnels:

- `TUNNEL_FAILED` — opening failed, or `list`/`close` could not confirm the
  OpenSSH master state. An unresponsive control socket is uncertain, not proof
  that the tunnel is stale or closed; preserve the record and retry or inspect
  before acting on that assumption. `TUNNEL_NOT_FOUND` means that id has no live
  record on this machine. Rediscover ids with `tunnel list`, and keep the
  canonical `data.tunnel_id` returned by `open` (`data.id` is a compatibility
  alias).

Input and adapter:

- `CONFIG_INVALID` — bad input: an invalid env var name or a destination that
  cannot be written.
- `USAGE_ERROR` — the command line itself was invalid (missing host, unknown
  flag). Fix the invocation; never retryable.
- `INTERNAL` — adapter bug. Report it with the JSON envelope attached.

## Recipes

**A session command timed out.** `data.session_preserved` is the answer: true
means the pane came back to a prompt; false means something still holds it.
`session read --since 0` shows what it printed, and `session recover` interrupts
it. Never follow a timeout with another blind command into the same pane.

**A REPL or debugger is running in a session.** That is what `session send` is
for (`--data 'next()' --enter`, `--key C-c`); `--data` is verbatim and
does not interpret `\n`. `session exec` will keep refusing with
`SESSION_BUSY` while it owns the pane, and that refusal is the guard rail, not a
failure to work around.

**A long command must survive the agent runtime.** Use `rhost exec` to submit it
to a scheduler already installed on the remote host, then use that scheduler's
own status, log, cancellation, and retention interfaces. rhost does not choose,
install, or unify schedulers.

**Everything fails with "no diagnostic".** Re-run one command with
`RHOST_SSH_LOG_LEVEL=VERBOSE`; rhost keeps OpenSSH quiet by default.

**You need to know what rhost did earlier.** `rhost audit --json` reads the local
trail (bounded operation metadata, never environment maps or file contents). It
is the only record of operations this machine performed; the remote side has
none of its own.
