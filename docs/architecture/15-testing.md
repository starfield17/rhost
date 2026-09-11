[← Architecture map](../ARCHITECTURE.md)

# Part XV — testing

## 40. Test layers

### Unit tests

Cover:

- shell quoting;
- OpenSSH argument construction;
- JSON schemas;
- status parsing;
- tmux command construction;
- session marker parser;
- byte-offset log cursors;
- job state machine;
- metadata serialization;
- dangerous sync-path rejection.

### Local integration tests

Use a disposable local SSH server/container when practical to test:

- OpenSSH invocation;
- ControlMaster reuse;
- stdout/stderr separation;
- non-zero exit;
- timeout;
- upload/download.

### Live remote integration tests

The critical features require a real remote Linux host.

Gate them behind explicit configuration, for example:

```text
RHOST_TEST_LIVE=1
RHOST_TEST_HOST=gpu
```

Never make ordinary unit tests unexpectedly touch a real machine.

Live tests must drive the **built binary as a child process**, one process per
step, never the Go API in-process. An in-process test cannot distinguish
"rhost persisted this" from "rhost happened to keep it in a map until now", and
that distinction is the product (AGENTS.md §4).

| suite | command |
|---|---|
| exec, timeout, doctor, transport reuse | `make test-live` |
| session persistence | `make test-live-session` |
| job persistence, signals, log cursors | `make test-live-jobs` |
| file transfer, rsync plan, explicit `--delete` | `make test-live-fs` |
| status snapshot, watch stream, watch offline | `make test-live-status` |
| remote file ops, verified transfer, `exec-many`, tunnels, session recovery | `make test-live-tools` |
| everything | `make test-live-all` |

The embedded remote helper has its own suite in the language it runs in:
`internal/fileops/remote_test.py`, driven from Go by `remote_helper_test.go` so it
is part of `make test` and cannot drift quietly. On a machine with no usable
python3 it is reported as a named *skip*, because a missing suite must be visible
(§39); the live SSH suite still exercises the real remote path end to end.

---

## 41. Required persistence tests

These are product-defining.

### Test A — connection persistence

```text
process 1: rhost exec
process exits

process 2: rhost exec
process exits

prove both reused the same ControlMaster
```

Automated: `TestLiveTransportReuse`. It reads the master pid back out of OpenSSH
(`ssh -O check`) before and after the second process, so equal pids are positive
evidence of reuse rather than an absence of errors.

### Test B — session persistence

```text
rhost session create
rhost process exits

new process:
session exec "cd ..."
process exits

new process:
session exec "pwd"
prove cwd persisted
```

Then kill the local invoking process abruptly and repeat discovery.

Automated: `TestLiveSession`, one CLI process per step: `create`, `cd`, then
`pwd` in a later process; `export` then `echo $VAR`; a stable `$$` proving the
same pane; and finally `SIGKILL` of a mid-command client, after which the session
is still listed, the still-running remote command holds the writer lock
(`SESSION_UNHEALTHY`, retryable), and the session becomes usable again on its
own.

### Test C — job persistence

```text
rhost job start "sleep ...; emit output"
starting process exits
close SSH master if desired

later:
rhost job status
rhost job logs
prove job survived and is discoverable
```

Automated: `TestLiveJob`, one CLI process per step. After `job start` it closes
the ControlMaster through OpenSSH itself and asserts no master is left running,
so the next process must reconnect; that new process then reports the job
`running`, reads its output through the byte cursor, waits for `exited` with the
job's own `exit_code`, and finds it again in `job list` once terminal.

`TestLiveJobStopHarvestsProcessGroup` and `TestLiveJobKillEscalates` cover §46's
"remote process group can be stopped": children of the job are proven gone by
name after `stop`, a TERM-ignoring job is reported as still `running` rather than
pretending to be stopped, and `kill` then removes the group.

`TestLiveJobStaleIsNeverSuccess` kills the group behind rhost's back with a bare
`kill -9`, which is what a reboot or OOM kill looks like to a later process: no
exit-code file was ever written, so the state must be `stale` and `exit_code`
`-1`, never a success.

### Test D — network interruption

Start a session and a job, temporarily break the local connection, reconnect, and prove:

- session is rediscovered;
- job is rediscovered;
- monitor reconstructs state.

Manual only. Deliberately not automated: nothing in this repository should imply
that a script can cut a real network link on someone else's host, and a simulated
disconnection would not prove what the test claims to prove.

---

## 42. Prove boundary checks work

Use Go language boundaries first.

`internal/` should prevent backend internals leaking across modules.

If an additional dependency rule is added, intentionally violate it once and confirm the check fails.

A boundary rule that has never failed in a test is not trusted.

This principle is taken from the useful pattern in the provided repository-shaping Skill: machine-enforced boundaries are more valuable than prose-only rules.

---

