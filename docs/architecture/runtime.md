# Runtime and foreground execution

## Transport

rhost invokes the system OpenSSH client. Target strings pass through unchanged,
so aliases, users, ports, proxy jumps, keys and host-key policy come from
OpenSSH configuration.

A private ControlPath namespace lets independent rhost invocations reuse an
OpenSSH ControlMaster. Persistence lives in OpenSSH, not in Go memory. rhost
never disables host-key checking and never stores credentials.

The remote wrapper requires `bash`, `setsid` and a base64 decoder. The command
payload crosses the login shell as base64, then runs under `bash -lc` in its own
process group. This keeps arbitrary shell syntax intact and makes cleanup
addressable.

## Primary command

```bash
rhost --host <host> [--cwd '<remote-dir>'] [--env KEY=VALUE] -- '<command>'
```

The command is one exact shell string. Stdin is forwarded. Human stdout and
stderr stream independently, and the remote exit code is returned locally.
`--json` captures bounded stream prefixes in one envelope; truncation fields
report the true byte counts.

There is no default deadline on the primary direct command. `--timeout` adds
one explicitly.

## Timeout and cancellation

A deadline or signal stops local SSH and then attempts to kill the recorded
remote process group. Execution timeout always uses
`REMOTE_COMMAND_TIMEOUT`:

- `cleanup_confirmed:true`: the process group was observed and killed;
  `retryable:true`.
- `cleanup_confirmed:false`: the remote state is uncertain;
  `retryable:false`.

`SSH_UNREACHABLE` is reserved for SSH connectivity and transport failures.
Cancellation uses `REMOTE_COMMAND_CANCELLED`. Output sink failure uses
`OUTPUT_WRITE_FAILED` and triggers the same cleanup attempt.
