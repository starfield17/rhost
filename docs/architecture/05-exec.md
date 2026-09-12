[← Architecture map](../ARCHITECTURE.md)

# Part V — foreground execution

## 10. CLI surface

The primary interface adds a remote context to one existing shell command:

```bash
rhost --host <host> [--cwd <remote-directory>] [--env KEY=value] -- '<command>'
```

Exactly one string follows `--`. It is a shell program interpreted remotely;
rhost does not pretend to preserve an argv array after a local shell has already
parsed it.

Human mode connects local stdin and streams remote stdout/stderr. It has no
default execution deadline or output cap. JSON mode collects bounded stream
prefixes and emits exactly one versioned result document after completion.

The older `rhost exec <host> -- <command...>` surface remains for compatibility
and keeps its existing timeout and capture defaults. It calls the same execution
protocol but is not the interface taught to new agents.

The result records the remote status, independent stream byte counts, truncation,
duration, timeout/cancellation, and whether cleanup was confirmed. A non-zero
remote status is still a successfully completed adapter operation.

## 11. Foreground timeout and cancellation

A direct command waits until it finishes unless the caller supplies `--timeout`
or the local process receives SIGINT/SIGTERM. Either event terminates the local
SSH process and starts a separate bounded cleanup request for the recorded remote
process group.

`cleanup_confirmed` distinguishes a command we killed from “the control path ended
but remote state is uncertain.” An uncertain side effect is never retried
automatically. Work intentionally meant to outlive the connection uses `job`.

The stdout protocol uses unpredictable begin/completion markers. Human mode
parses them incrementally so command bytes are forwarded before completion while
protocol bytes and login-profile noise stay out of user output. JSON mode applies
the same boundaries while retaining only the requested prefix.
