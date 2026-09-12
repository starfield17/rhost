[← Architecture map](../ARCHITECTURE.md)

# Part XIX — Definition of done

## 53. First release

A coding agent can take a command it would run locally, add an SSH target and
remote working directory, and receive its input/output/status without learning a
second command vocabulary:

```bash
rhost --host <host> --cwd '/home/<user>/<project>' -- 'go test ./...'
```

Foreground cancellation cleans the remote process group or reports uncertainty.
Connection reuse survives the CLI. A task uses session, job, file, or tunnel
operations only when it needs persistent state, detached lifetime, cross-machine
transfer, protected editing, or forwarding.

All of these claims are verified against a real remote Linux host using a target
supplied at runtime through `RHOST_TEST_HOST`.

