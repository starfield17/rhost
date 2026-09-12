[← Architecture map](../ARCHITECTURE.md)

# Part XV — testing

## 40. Test layers

Unit tests cover shell quoting, the incremental execution protocol, bounded
capture, JSON schemas, OpenSSH arguments, tmux boundaries, job identity, file
hash protection, and dangerous sync target rejection.

Live tests require an explicit runtime target:

```bash
RHOST_TEST_HOST=<user>@<host> make test-live
RHOST_TEST_HOST=<user>@<host> make test-live-all
```

They run the built binary as a separate process per step. This is required to
prove that ControlMaster reuse, remote tmux sessions, detached jobs, and tunnel
records survive CLI process exit. Ordinary unit tests never select or contact a
real host.

| suite | coverage |
|---|---|
| `make test-live` | direct and compatibility exec, stdin, timeout, doctor, transport reuse |
| `make test-live-session` | remote tmux lifetime and recovery |
| `make test-live-jobs` | detached lifetime, signals, identity, log cursors |
| `make test-live-fs` | transfers, sync preview and deletion rules |
| `make test-live-tools` | safe editing, verified transfer, tunnels, session recovery |
| `make test-live-all` | every live test |

The embedded remote Python file helper has a local suite driven by Go, so it is
part of `make test`. Python 3 is a required local test dependency.

## 41. Required persistence tests

- Two separate CLI processes reuse the same OpenSSH ControlMaster.
- Session cwd, environment, and pane identity survive process exit and reconnect.
- A detached job survives the launching CLI and a closed SSH connection, then is
  rediscovered with its output and real exit code.
- Stops and kills target only a process group whose recorded identity still
  matches; stale pids are never signalled.
- Tunnels remain owned by their dedicated OpenSSH master after the opener exits.

Foreground execution separately proves that stdin crosses the connection,
stdout is observable before command completion, JSON remains one document,
timeouts/signals clean the remote process group, and uncertain cleanup is
reported rather than hidden.

## 42. Prove boundary checks work

Use Go `internal/` boundaries first. Any additional dependency rule must be
intentionally violated once and observed failing before it is trusted.
