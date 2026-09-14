# Runtime and foreground execution

## Transport

rhost invokes the system OpenSSH client. Target strings pass through unchanged,
so aliases, users, ports, proxy jumps, keys and host-key policy come from
OpenSSH configuration.

A private ControlPath namespace lets independent rhost invocations reuse an
OpenSSH ControlMaster. Persistence lives in OpenSSH, not in CLI memory. rhost
never disables host-key checking and never stores credentials.

`connection status` and `connection reset` expose that shared master's control
state without reimplementing SSH configuration. Reset uses `ssh -O stop`, so it
does not terminate already accepted channels. `--fresh` on exec and doctor uses
`ControlMaster=no`, `ControlPersist=no`, and `ControlPath=none`, and does not
initialize the shared socket directory. Tunnel masters use a separate namespace.

The remote wrapper requires `bash`, `setsid` and a base64 decoder. The command
payload crosses the login shell as base64, then runs under `bash -lc` in its own
process group. This keeps arbitrary shell syntax intact and makes cleanup
addressable.

## Primary command

```bash
rhost exec <host> [--cwd '<remote-dir>'] [--env KEY=VALUE] (--command '<shell-program>' | --command-file <local-file>)
```

The command is one exact, non-empty shell program. Stdin is forwarded. Human
stdout and stderr stream independently, and the remote exit code is returned locally.
`--json` captures bounded stream prefixes in one envelope; truncation fields
report the true byte counts.
With `--json --stream`, both live remote streams are mirrored to local stderr
while stdout remains the final JSON document. Capture continues independently
and may truncate without stopping the live mirror.

There is no default deadline on the primary direct command. `--timeout` adds
one explicitly.

## Timeout and cancellation

A deadline or signal stops local SSH and then attempts to kill the recorded
remote process group. Execution timeout always uses
`REMOTE_COMMAND_TIMEOUT`. The v2 envelope reports cleanup separately:

- `data.cleanup.status == "confirmed_stopped"`: the recorded process identity
  was matched and its managed process group was confirmed empty; side effects
  and detached work are outside that claim.
- `data.cleanup.status == "unconfirmed"`: the managed group was not proven
  stopped. Both states remain `error.retryable:false`.
- `data.cleanup.status == "not_attempted"`: no cleanup was warranted, including
  when foreground completion had already been observed.

Both timeout states are non-retryable until the caller checks the operation's
actual effect. Missing completion evidence without a specific connection-stage
diagnosis is `REMOTE_EXECUTION_UNKNOWN`, not `SSH_UNREACHABLE`.

`SSH_UNREACHABLE` is reserved for SSH connectivity and transport failures.
Cancellation uses `REMOTE_COMMAND_CANCELLED`. Output sink failure uses
`OUTPUT_WRITE_FAILED` and triggers the same cleanup attempt.

The wrapper, cleanup command, doctor and session helpers resolve the same remote
v4 state root: `${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost/v4}`. An explicit
`RHOST_REMOTE_STATE` is already the complete root and is not suffixed again.
