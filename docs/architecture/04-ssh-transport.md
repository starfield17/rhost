[← Architecture map](../ARCHITECTURE.md)

# Part IV — SSH transport

## 7. v0.1 transport: system OpenSSH

The first implementation should intentionally use the system `ssh` binary.

Reasons:

- exact compatibility with existing `~/.ssh/config`;
- uses the operator's ssh-agent and keychain behavior;
- mature host-key verification;
- ProxyJump and uncommon SSH options work without reimplementation;
- ControlMaster provides connection persistence outside the CLI process;
- simplest path to a reliable first release on the primary client platform.

The single release binary therefore orchestrates OpenSSH; it does not need to reimplement the SSH protocol in v0.1.

### Later option

A native Go transport using `golang.org/x/crypto/ssh` may be added if there is a concrete need for:

- no external OpenSSH dependency;
- Windows-native local support;
- tighter streaming control;
- a future daemon;
- multiplexing not expressible cleanly through OpenSSH.

Do not build both backends in the first release.

---

## 8. ControlMaster management

`rhost` should maintain its own socket namespace, for example:

```text
~/.cache/rhost/ssh/
    <hash>.sock
```

Use a hashed path because Unix-domain socket paths have length limits.

That limit is a hard budget, not a style preference: `sun_path` allows 104 bytes
including the NUL on BSD-derived systems (108 on Linux), and OpenSSH substitutes a
fixed 40-character digest for `%C`. A deep cache root therefore breaks *every*
command with `ControlPath too long` - reachable through `RHOST_CACHE_DIR` or
`$XDG_CACHE_HOME`, which is how this was first hit. `config.ControlDir()` checks
the expanded length and falls back to a short per-user root (`/tmp/rhost-<uid>/ssh`)
when the cache root is too deep; it must not use `os.TempDir()` for that fallback,
because on macOS the per-session temp dir is itself long. The directory rhost
creates and the directory it binds in must come from the same function.

Conceptual options:

```text
ControlMaster=auto
ControlPersist=15m
ControlPath=~/.cache/rhost/ssh/%C
BatchMode=yes
```

Operations:

```text
ensure master exists
check master
open command channel
allow ControlPersist to keep transport alive
explicit close only when requested / cleanup requires it
```

The transport lifetime is not the CLI lifetime.

### Required test

Start one master, run multiple `rhost exec` invocations as separate OS processes, and prove they reuse the same master.

Do not infer this from timing alone. Use an observable OpenSSH control check or debug evidence in integration tests.

### Tunnels are a second namespace of masters (implemented)

`rhost tunnel open` starts its *own* OpenSSH process with its own control socket
and `-N` (no remote command), because a port forward has to outlive the command
that asked for it while staying independent of the shared one:

- the shared master is opened with `ControlPersist`, so it is *idle*-timed and
  closing it is nobody's business per-command; a tunnel's master has no such
  margin — it is closed by `tunnel close <id>`, by an explicit kill of its own
  socket, or not at all;
- `tunnel close` must never take the shared socket down, since that would stop
  `exec`, `session` and `job` traffic for every other caller. The two namespaces
  are therefore separate directories under the same state root, and the shared
  socket name is derived from the target by the same rule `exec` uses, so a
  tunnel does not "discover" it;
- the record of a tunnel (id, host, kind, bind, destination) lives in a file, not
  in memory: the CLI process that opened it is gone by the time anyone wants to
  close it (AGENTS.md §4). `list` rediscoveres the directory and asks OpenSSH
  `-O check` for each entry rather than trusting the file's own claim.

A forward is OpenSSH's, not rhost's: what `sshd` permits (`GatewayPorts`,
`permitreverse`, the address a bind may use) is answered by OpenSSH's stderr,
quoted verbatim under the taxonomy's `TUNNEL_FAILED`. rhost adds one local rule —
loopback unless `--allow-exposure` — because that is the one decision that would
otherwise be made silently on the caller's behalf.

---

## 9. Command construction

Avoid accidental dependence on the remote account's default shell.

For normal Linux execution use a controlled shell, initially:

```text
bash -lc <command>
```

`cwd` and environment must be encoded explicitly.

Conceptually:

```bash
cd -- "$cwd"
export KEY=...
exec-or-run command
```

Do not rely on a previous `cd` or `export`.

All shell quoting/escaping must live in one module with tests.

Never build remote commands by naive string concatenation across the codebase.

---

