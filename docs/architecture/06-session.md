[← Architecture map](../ARCHITECTURE.md)

# Part VI — `session`

## 12. Why tmux is the v0.1 session backend

A persistent session must survive:

- one `rhost` invocation ending;
- the agent process ending;
- SSH disconnect;
- temporary network loss;
- a later `rhost` invocation.

Remote tmux already owns exactly this lifetime.

Therefore:

```text
rhost CLI
   │
   ├─ create/list/send/read/attach/close
   │
   ▼
SSH
   │
   ▼
remote tmux session
   │
   ▼
managed shell / REPL / debugger
```

Do not store a live PTY object only in local memory.

---

## 13. Session identity and remote layout

Namespace all managed sessions.

Example tmux session name:

```text
rhost_s_<short-id>
```

Remote metadata:

```text
~/.local/state/rhost/
└── sessions/
    └── s_01J.../
        ├── meta.json
        └── pty.log
```

`meta.json` example:

```json
{
  "schema_version": 1,
  "id": "s_01J...",
  "name": "debug",
  "tmux_session": "rhost_s_01J...",
  "created_at": "2026-09-10T12:00:00Z",
  "created_by": "rhost",
  "initial_cwd": "/home/dev/work/foo",
  "shell": "bash"
}
```

The remote state is authoritative for discovery.

If metadata exists but tmux does not, report the session as stale/dead. Do not pretend it is alive.

---

## 14. Session creation

Prefer launching a known shell explicitly, for example:

```text
bash --noprofile --norc -i
```

or a configurable login-shell mode when needed.

Do not blindly inherit fish/zsh/bash differences and then make the command protocol guess them.

The session backend should set an adequate tmux history limit.

Creation holds one remote `flock` across duplicate-name detection, tmux startup,
and metadata publication. This makes same-name concurrent creates deterministic.
Until metadata is durable, an exit/signal trap owns cleanup of the new tmux
session and state directory, so a dropped SSH channel does not leave a hidden
session outside discovery.

Attach a pane output log using tmux `pipe-pane` or an equivalent mechanism so output can be read incrementally after reconnect.

The exact mechanism must be integration-tested from a real client host against a
real remote Linux host.

`session attach` is intentionally human-only. Combining it with `--json` fails
locally with `USAGE_ERROR`; interactive terminal bytes can never masquerade as a
JSON envelope.

---

## 15. Session command modes

A session needs **two** kinds of input.

### `session exec`

For a normal shell command where the caller wants:

```text
command
→ output
→ exit code
→ command finished
```

Example:

```bash
rhost session exec gpu debug -- 'cd ~/work/foo'
```

This path should use a command-boundary protocol.

### `session send`

For raw interactive input where no shell command boundary is expected:

```bash
rhost session send gpu debug --data 'next\n'
rhost session send gpu debug --key C-c
```

Use this for:

- Python REPL;
- gdb/lldb/pdb;
- interactive installers;
- TUI tools;
- Ctrl-C and other control keys.

Do not force `session exec` semantics onto arbitrary interactive programs.
`session exec` enforces that distinction rather than leaving it to the caller: it
refuses with `SESSION_BUSY` while any program other than the managed shell is the
pane's foreground process, and the refusal names that program. The refused
command never ran. `session send` remains the raw path, and `session recover` is
the explicit way to interrupt a program that should not be there.

---

## 16. Reliable input injection

Do not implement arbitrary commands by embedding them directly in:

```text
tmux send-keys "...user text..."
```

That creates quoting and special-key problems.

Prefer:

```text
local command bytes
    ↓
encode/transfer safely
    ↓
tmux load-buffer
    ↓
tmux paste-buffer
    ↓
send Enter separately
```

or an equivalent binary-safe path.

Raw keys such as `C-c` should use explicit tmux key operations.

Before a managed `session exec`, the backend may need to normalize the shell input line (for example Ctrl-C/Ctrl-U) only when it is known to be at a managed shell prompt. Do not send destructive control sequences blindly while an interactive program owns the foreground.

The check is mechanical: `tmux display-message -p '#{pane_current_command}'` must
equal the shell recorded in the session's `meta.json`, and it happens under the
writer lock, before the first byte of input. The idle probe that follows (which
itself types `stty -echo`) is therefore never delivered to a program that is not
the shell.

Terminal *settings* are not input. Echo, for example, is a property of the pane's
pty: read `#{pane_tty}` and apply `stty echo` / `stty -echo` to that device, never
by typing the command into the pane, which would deliver it to whatever owns the
foreground and change nothing about the terminal.

---

## 17. Command-boundary protocol

This is one of the places where Codex should inspect the local `portal-mcp-server` source.

Portal's persistent-shell design is useful reference material for:

- marking command completion in a PTY stream;
- recovering the remote exit code;
- avoiding prompt-string guessing;
- soft-cancel behavior;
- resynchronizing after an interactive prompt;
- serializing access to one shell.

For `rhost`, adapt the idea to a tmux-owned session.

Two acceptable approaches:

### Preferred if verified through tmux

Use an OSC 133-style completion marker carrying the exit status.

### Simpler fallback

Use a cryptographically random per-command nonce:

```text
__RHOST_DONE_<128-bit-random>__:<exit-code>
```

The marker is written only after the command returns.

The parser reads the session log until it observes that exact nonce.

Do not use a fixed sentinel such as `DONE`.

Whichever protocol is selected, add an integration test proving:

- normal output may contain arbitrary similar strings;
- UTF-8 boundaries do not corrupt parsing;
- non-zero exit codes are returned;
- timeout/cancel leaves the shell in a known state or marks the session unhealthy;
- separate CLI processes can continue the same session.

---

## 18. Incremental session reads

Target:

```bash
rhost session read gpu debug --since 18422 --json
```

Result:

```json
{
  "session_id": "s_01J...",
  "from": 18422,
  "next": 19284,
  "data": "...",
  "more": false
}
```

Use a byte offset or another explicit cursor.

Do not make the agent repeatedly parse a full `tmux capture-pane` snapshot.

A full-screen capture command may still exist for humans/debugging, but incremental log reading should be the machine-facing primitive.

---

## 19. Human attach

A human should be able to enter the exact same remote terminal:

```bash
rhost session attach gpu debug
```

Implementation can simply exec an interactive SSH command equivalent to:

```text
ssh -t <host> tmux attach-session -t <managed-session>
```

This is a core reason to keep sessions remote and tmux-backed.

Human attach and agent interaction must target the same tmux pane/session.

Concurrency policy for simultaneous human and agent input should initially be simple and explicit:

- multiple readers are fine;
- concurrent writers are unsafe;
- `session exec` must hold a logical session lock;
- human attach should warn that automated writes can race;
- sophisticated ownership/takeover modes may be added later.

Do not build a collaborative terminal protocol in v0.1.

---
