# rhost project overview

## Intent

A coding agent already knows how to inspect and change a local machine with shell
commands. rhost carries that model across SSH:

```bash
rhost --host gpu -- 'uname -a'
rhost --host gpu --cwd '~/project' -- 'git status --short'
rhost --host gpu -- 'find . -maxdepth 2 -type f' | sort
```

The quoted command belongs to the remote shell. The pipeline after rhost belongs
to the local shell. This boundary is the center of the product.

## Capabilities

- `doctor` returns platform facts and a capability matrix in one round trip.
- `session` keeps shell state in remote tmux across independent CLI processes.
- `job` starts and rediscovers detached remote processes.
- `fs` moves files and provides bounded, hash-guarded text editing.
- `tunnel` manages OpenSSH forwards.
- `audit` records bounded local operation metadata.
- `--json` emits one versioned envelope with stable error codes.

## Ownership

The rhost process owns no durable remote work. OpenSSH ControlMaster owns
connection reuse; remote tmux owns sessions; remote process groups and state
files own jobs; dedicated OpenSSH masters own tunnels. Authentication and host
key decisions always remain with OpenSSH.

## Agent workflow

Start with `doctor`, then choose the smallest primitive that matches the work.
Combine related read-only observations into one remote command to avoid repeated
SSH round trips. Keep unrelated mutations separate so each exit status and retry
decision remains clear.

Use `data.exit_code` for a completed command and `error.code` for adapter
failures. After a timeout, inspect `data.cleanup_confirmed`; do not repeat a
side effect when cleanup was not confirmed.
