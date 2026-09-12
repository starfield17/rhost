[← Architecture map](../ARCHITECTURE.md)

# Part III — Host/configuration model

## 5. OpenSSH config is the authentication source of truth

Do not create a parallel SSH credential format.

Expected user configuration:

```sshconfig
Host gpu
    HostName gpu.example.internal
    User dev
    IdentityFile ~/.ssh/id_ed25519
    ServerAliveInterval 30
    ServerAliveCountMax 3
```

Use placeholder names in documentation. Real addresses, accounts, and personal
SSH aliases must not appear in tracked files (`AGENTS.md` §1).

`rhost` should invoke the user's OpenSSH client with `Host=gpu` semantics so that it naturally inherits:

- `HostName`;
- `User`;
- `Port`;
- `IdentityFile`;
- ssh-agent;
- `ProxyJump`;
- `Include`;
- `known_hosts`;
- host key policy.

There is no required rhost host registry or default-host configuration. Add
adapter configuration only when a setting cannot belong to the invocation,
OpenSSH, or the remote environment. Never put private keys or passwords there.

---

## 6. Host capability discovery

`rhost doctor <host>` should probe and return capabilities rather than silently assuming them.

Minimum probes:

```text
SSH reachable
batch authentication works
bash available
tmux available
nohup available
setsid available
ps available
remote state dir writable
rsync available locally
rsync available remotely
WSL detected or not
```

Example human output:

```text
gpu
SSH                 OK
Host key            verified by OpenSSH
bash                OK
tmux                OK
durable jobs        OK (nohup + setsid)
rsync               OK
NVIDIA telemetry    OK
WSL2                yes
```

Example JSON should expose the same facts mechanically.

Do not auto-install missing packages in v0.1.

---
