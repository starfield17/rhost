# rhost CLI cookbook

Every command below takes `--json`; the JSON envelope is the contract, and
[`schemas/result-v1.schema.json`](../../../schemas/result-v1.schema.json) is its
schema. This file is the command surface, not the policy: start from
[../SKILL.md](../SKILL.md).

Quoting note: remote paths beginning with `~` must be quoted (`'~/work/foo'`), or
the local shell expands its own home directory.

## Finding hosts

```bash
rhost hosts --json
```

Any target you can name to `ssh` also works directly as the host argument: an
alias from `~/.ssh/config` (`gpu`), `user@host`, or a bare hostname.

`hosts` is **best-effort alias discovery**, not the authority on what OpenSSH
would resolve: it reads `~/.ssh/config` and its `Include` files, and does not
implement `Match`, quoted includes, or the system-wide config. A host you can
reach may be missing from the list, and a listed alias is not a promise that the
connection will succeed. `ssh <target>` decides.

## Probing a host

```bash
rhost doctor <host> --json
```

Reports OS, kernel, arch, user, login shell, whether the remote state directory is
writable, and which of `bash tmux nohup setsid ps rsync sha256sum base64 stty
nvidia-smi systemctl git flock python3 rg realpath` exist — as
`data.capabilities`. `data.capabilities.pid_identity` is separate: it says
whether `/proc` gives the job backend the process identity it needs.

Do this once per host before expensive work, and again after a host is rebuilt.

## Foreground execution: `exec`

```bash
rhost exec <host> --json --cwd '~/work/foo' --timeout 60s -- pytest -q
rhost exec <host> --json --env CUDA_VISIBLE_DEVICES=0 -- python check.py
rhost exec <host> --json -- 'echo hi | wc -l'
```

- Stateless: no cwd, env, or shell state carries over. Set them per call.
- The words after the host are joined with spaces and run by a login `bash`, so
  shell syntax works. Use `--` when the command could contain rhost flags.
- Output defaults to 1 MiB per stream. `data.stdout_truncated` /
  `data.stderr_truncated` say whether a stream was cut, and the byte counts are
  reported; `--max-output-bytes 0` removes the bound. A truncated result still
  proves the command finished.
- On timeout the remote process group is killed and `data.cleanup_confirmed`
  says whether that could be confirmed. If it is false, the command may still be
  running: do not blindly repeat a side effect.
- A long command belongs in a `job`, not in an ever-larger `--timeout`.

## Persistent sessions: `session`

Use a session only when state must persist — a REPL, a debugger, an interactive
CLI, exploratory `cd`/`export` work. Otherwise `exec`.

```bash
rhost session create <host> --json --name debug --cwd '~/work/foo'
rhost session list <host> --json
rhost session exec <host> debug --json -- 'cd src && pytest -q'
rhost session exec <host> debug --json -- pwd        # cwd persisted
rhost session send <host> debug --data 'next()\n'    # raw input, for REPLs
rhost session send <host> debug --key C-c            # one control key
rhost session read <host> debug --json --since 0     # incremental log
rhost session attach <host> debug                    # human only
rhost session recover <host> debug --json            # stop a wedged program
rhost session close <host> debug
```

- The session is owned by **remote tmux**: it survives this process and the SSH
  connection. Rediscover it with `session list`; never assume it vanished.
- `session exec` persists shell state and returns the command's real exit code in
  `data.exit_code`. It wraps the command so exactly one command boundary is
  produced.
- **`session exec` vs `session send`.** `exec` is for the managed shell and
  refuses with `SESSION_BUSY` when anything else owns the pane; `send` is raw
  terminal input and types into whatever is running. A command that was refused
  never ran. To see what a busy pane is doing, `session read`; to interrupt it,
  `session recover`.
- `session exec` on a command that runs `exit` ends the session and reports
  `SESSION_NOT_FOUND`. Recreate it if that was not the intent.
- `session read` is a byte cursor: pass `data.next` back as `--since` to tail
  without re-reading. Output is ANSI-stripped for you.
- Writers are serialised by a remote lock, but a human attached at the same time
  can still race an agent. Agents should not use `attach`.
- After a timeout, `data.session_preserved` says whether the pane was observed
  returning to a prompt. False means something is still holding it.

## Detached jobs: `job`

Use a job when the work must outlive the connection: a training run, a long
build, a benchmark. A job has no shell — a command, two log files, and an exit
status.

```bash
rhost job start <host> --json --cwd '~/work/foo' -- python train.py
rhost job start <host> --json --name train-1 --env CUDA_VISIBLE_DEVICES=0 -- ./train.sh
rhost job list <host> --json
rhost job status <host> <job-id> --json
rhost job logs <host> <job-id> --json --stream stdout --since 0
rhost job stop <host> <job-id> --json     # SIGTERM to the whole process group
rhost job kill <host> <job-id> --json     # SIGKILL, for jobs that ignore TERM
```

- `job start` answers `{id, state, pid}` immediately. **Keep the id** (`j_` plus
  12 hex digits): a `--name` works for the other commands only while it matches
  exactly one job, and a name matching several is refused with `CONFIG_INVALID`
  rather than guessed. A `--name` must not look like a generated id.
- A failed or timed-out launch can still carry `{id, state:"unknown", pid:0}` in
  the failure envelope. Query that id before retrying so a surviving detached
  launch is not duplicated.
- The job is a detached remote process plus remote files, so it survives the CLI
  and the connection. `job list` rediscovers it; nothing is remembered locally.
- `job logs` is a byte cursor like `session read`. `data.data` is **base64**
  (`data.encoding` says so) because logs are arbitrary bytes: decode it before
  searching, and pass `data.next` back as `--since`.
- States are `starting`, `running`, `exited`, `failed`, `stopped`, `stale`.
  - `exited` / `failed`: the job wrote its own exit code; read `data.exit_code`.
  - `stopped`: a stop or kill was requested and the process is gone. A job
    terminated by its own stop records `143` (128+SIGTERM); `job kill` leaves
    `data.exit_code: -1` because SIGKILL cannot be trapped and rhost will not
    invent a status.
  - `stale`: the recorded process is gone, or its pid now belongs to a different
    process. **Never a success** — treat the result as unknown and re-run the
    work if it mattered.
- `running` requires a verified process identity: the boot id and process start
  time recorded at launch still describe the live pid. `job stop` / `job kill`
  report `data.signalled` — `false` means nothing was signalled because the
  process group could not be tied to the job, and the state is the answer.
- Stop and kill are idempotent, they signal the whole process group (children
  included), and they never rewrite an exit code that was already recorded. A job
  that ignores SIGTERM is reported as still `running` after the grace period:
  escalate with `job kill`.
- Finished jobs stay inspectable. There is no `job rm`: cleanup means deleting
  the remote state directory.

## Files: `fs`

```bash
rhost fs put <host> ./model.py '~/work/foo/model.py' --json
rhost fs get <host> '~/work/foo/results.json' ./results.json --json
rhost fs sync <host> ./project '~/work/project' --json --dry-run
rhost fs sync <host> ./project '~/work/project' --json
rhost fs sync <host> ./project '~/work/project' --json --delete --dry-run
```

- `put`/`get` copy one file (scp); `sync` copies a directory tree (rsync).
  Handing a directory to `put` is `CONFIG_INVALID` pointing at `sync`, never a
  silent recursive copy.
- `sync` never deletes unless `--delete` is given, and `--delete` is refused
  (`SYNC_REJECTED`) for a top-level directory or a whole home. There is no flag
  that makes that safe: sync into a subdirectory.
- **Read the plan before applying it.** `--dry-run` returns the same
  `data.changes` list as the real sync (`action`: `create`, `update`, `delete`,
  `directory`, `skip`, with the raw rsync itemize string beside it).
  `data.deletes` counts the destructive part. An empty list means the two sides
  already match.
- A destination may not contain glob characters: the remote shell would expand
  them. A single-file remote source may not contain them either. Trailing slashes
  follow rsync's convention; rhost normalises the source.
- `data.multiplexed` is true only when OpenSSH's shared master answers after the
  transfer. `false` means persistent reuse was not observed; the copy may still
  be correct.
- Transfers are **foreground and owned by this process**: killing the CLI stops
  the copy. Nothing about `fs` persists.
- No rsync on the remote is `REMOTE_DEPENDENCY_MISSING`, and the message says that
  `put`/`get` still work. `rhost doctor` reports it up front.
- Errors: `TRANSFER_FAILED` (the tool's own complaint is in `message`),
  `SYNC_REJECTED`, `REMOTE_DEPENDENCY_MISSING`, `REMOTE_COMMAND_TIMEOUT`.

### Reading and editing remote files

```bash
rhost fs read  <host> '~/work/project/main.go' --json    # slice + whole-file sha256
rhost fs write <host> '~/work/foo/note.txt' --from ./note.txt --if-hash <sha> --json
rhost fs patch <host> '~/work/project/main.go' --patch ./patch.json --json
rhost fs grep  <host> 'TODO' '~/work/project' --mode content --limit 100 --json
rhost fs glob  <host> '*.go' '~/work/project' --json
rhost fs mirror <host> '~/work/project' ./download --dry-run --json
rhost fs batch  <host> --manifest ./transfers.json --json
```

- These ride an embedded Python helper on the far side: they need remote
  `python3`, and the two searches also need `rg`. Nothing is installed for you.
- **Read before you write.** `fs read` returns the SHA-256 of the whole file, and
  writing or patching an existing file without that hash is refused. A hash you
  did not just read is a guess about the file's contents.
- A new file is created `0600` and an existing one keeps its permissions; pass
  `--mode 0755` to name them, which is how a written script becomes runnable.
- Reads and searches are bounded (`truncated` is in the JSON, `next` is the
  offset to pass back as `--offset`). Pagination is by records, not bytes.
- `fs mirror` is `sync` with the direction in the name; `fs batch` is an ordered
  manifest of `put`/`get` pairs. Both keep per-item results: check every row,
  because the aggregate exit code says only that something failed.
- Edits are UTF-8 text, limited to 8 MiB, and are never applied through a symlink.
- Patch JSON uses 1-based inclusive line ranges against the original file:
  `{"sha256":"<hash-from-read>","edits":[{"start":2,"end":3,"text":"replacement\n"}]}`

## Host state: `status` and `watch`

```bash
rhost status <host> --json              # one snapshot
rhost watch <host>                      # human, refreshes every 2s until Ctrl-C
rhost watch <host> --interval 5s
rhost watch <host> --json --count 1     # one envelope, for a scripted probe
```

- `status` returns one read-only snapshot: OS, kernel, arch, uptime, load, CPU,
  memory, disk, accelerators, and the sessions and jobs rhost manages on that
  host.
- A metric the probe could not read is `null` and named in `data.unavailable`.
  **Never read `null` as zero.** An empty `data.accelerators` means no GPU was
  found, not a failure.
- `data.online` is reachability and `data.probe_ms` is how long the snapshot
  took; it is deliberately not a network RTT.
- `watch` owns no state: every refresh re-reads the host and rediscovers its
  sessions and jobs. A dropped connection shows as `data.online: false` with
  `data.offline_code`, and the next good refresh reconstructs everything from the
  host.
- `watch --json` prints one envelope per line (NDJSON), so parse it incrementally.

## Many hosts and forwards: `exec-many`, `tunnel`

```bash
rhost exec-many --host a --host b --parallel 2 --json -- 'uname -a'
rhost exec-many --host a --host b --serial --delay 2s --stop-on-error --json -- ./deploy.sh
rhost tunnel open <host> --kind local --listen localhost:8080 --destination localhost:8000 --json
rhost tunnel open <host> --kind reverse --listen localhost:9000 --destination localhost:3000 --json
rhost tunnel open <host> --kind socks --listen localhost:1080 --json
rhost tunnel list --json
rhost tunnel close <id> --json
```

- `exec-many` runs one command on several targets and returns one row each, in the
  order the flags were given. It is foreground orchestration, not a scheduler: if
  it must survive this machine closing, start a `job` per host.
- `--stop-on-error` stops *scheduling* new targets; a command already running on
  another host is not cancelled. `--delay` is per worker, so `--serial --delay 5s`
  is a five-second gap between targets.
- Read each row's `error` and `exit_code`: the process exit status is an
  aggregate (0 all good, 1 a remote failure or a skipped target, 255 an adapter
  failure) and says nothing about which host.
- A tunnel is a **dedicated OpenSSH master with a record outside this process**:
  it keeps running after the CLI exits, and `tunnel list` is how you find ids you
  did not keep. It is not a service check: `alive` means the forward exists, not
  that anything answers behind it. Probe the port yourself.
- Forwards bind to loopback unless `--allow-exposure` says otherwise, which is a
  request to let other machines through. The remote `sshd` still has its own say,
  and a denied bind is reported as OpenSSH reported it.
