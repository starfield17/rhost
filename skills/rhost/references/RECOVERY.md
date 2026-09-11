# Recovering from a failed rhost call

Read `ok`, `error.code`, `error.retryable` and the operation's `data` from the
JSON envelope. The message is for the human reading the transcript; the code is
what to branch on. Half of these answers are "the session/job is fine, use the
other command".

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
  what still works without it. For jobs this includes `bash`, `setsid`, `nohup`
  and a readable `/proc`; for sessions `tmux` and `flock`; for `fs sync` `rsync`;
  for `fs read/write/patch/grep/glob` `python3` (and `rg` for the searches).
  rhost never installs anything for you.
- `UNSUPPORTED_REMOTE_OS` — the probe could not establish a supported platform.

Time:

- `REMOTE_COMMAND_TIMEOUT` — the deadline passed and the remote process group was
  killed. For `exec`, check `data.cleanup_confirmed`: if it is false the command
  may still be running, so do not repeat a side effect before checking. For a
  session, check `data.session_preserved`: false means something still holds the
  pane (`session read` to see what, `session recover` to interrupt it). Work that
  should outlive a timeout belongs in a `job`.

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

Jobs:

- `JOB_NOT_FOUND` — no job with that id (or unique name) on that host. It may be
  from another host, or the state was deleted. `job list` rediscovers.
- `JOB_STATE_UNKNOWN` — the job exists but its state could not be read or
  written: the state directory is unwritable, or a helper died mid-write. The job
  itself is usually unaffected; retry, and inspect the host if it repeats.
- a `state` of `stale` is not an error code but the same kind of answer: the
  recorded process is gone or its pid now belongs to a different process.
  Nothing was signalled (`data.signalled` is false), the result is unknown, and
  re-running the work is the only honest next step.

Files:

- `FILE_NOT_FOUND` — the remote path does not exist. The local shell cannot stat
  a remote path, so resolve it with `fs glob` instead of guessing.
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
- `SEARCH_FAILED` — the remote `rg` refused the pattern or could not read the
  tree. This is a failed command, not a connection problem.
- `TRANSFER_FAILED` — the transfer tool said no; its own first complaint is in
  `message`. `SYNC_REJECTED` is the subset that rhost refused before running
  anything: a destination that is a glob, a whole home, or a top-level directory
  with `--delete`.
- `fs batch` keeps per-entry results in `data.results[]` even when the run fails
  overall: the aggregate exit status does not name which entry failed.

Tunnels:

- `TUNNEL_FAILED` — the forward could not be opened; OpenSSH's reason is in the
  message. `TUNNEL_NOT_FOUND` — that id has no live record on this machine.
  `tunnel list` is how ids are rediscovered; keep `data.id` from `open`.

Input and adapter:

- `CONFIG_INVALID` — bad input: an invalid env var name, a job handle shaped like
  shell syntax, an ambiguous job name, a destination that cannot be written.
- `USAGE_ERROR` — the command line itself was invalid (missing host, unknown
  flag). Fix the invocation; never retryable.
- `INTERNAL` — adapter bug. Report it with the JSON envelope attached.

## Recipes

**A session command timed out.** `data.session_preserved` is the answer: true
means the pane came back to a prompt; false means something still holds it.
`session read --since 0` shows what it printed, and `session recover` interrupts
it. Never follow a timeout with another blind command into the same pane.

**A REPL or debugger is running in a session.** That is what `session send` is
for (`--data 'next()\n'`, `--key C-c`); `session exec` will keep refusing with
`SESSION_BUSY` while it owns the pane, and that refusal is the guard rail, not a
failure to work around.

**The connection dropped mid-job.** Nothing was lost: jobs are detached remote
processes. `job list`, then `job status`/`job logs --since <cursor>`.

**A job shows `stale`.** The process is gone, or the pid belongs to something
else now. Read its logs for what happened, then re-run if the work mattered.

**Everything fails with "no diagnostic".** Re-run one command with
`RHOST_SSH_LOG_LEVEL=VERBOSE`; rhost keeps OpenSSH quiet by default.

**You need to know what rhost did earlier.** `rhost audit --json` reads the local
trail (operations, not secrets). It is the only record of operations this machine
performed; the remote side has none of its own.
