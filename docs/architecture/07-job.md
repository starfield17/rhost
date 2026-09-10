[← Architecture map](../ARCHITECTURE.md)

# Part VII — `job`

## 20. Job semantics

Target:

```bash
rhost job start <host> [flags] -- <command...>
```

Example:

```bash
rhost job start gpu \
  --cwd ~/work/foo \
  -- python train.py
```

Result:

```json
{
  "job_id": "j_01J...",
  "state": "running",
  "pid": 18372
}
```

The command continues after:

- `rhost` exits;
- the starting terminal closes;
- SSH disconnects.

It does not promise survival of a remote OS shutdown or WSL shutdown.

---

## 21. Remote job state layout

Use persistent per-user state, not `/tmp`, unless there is a strong reason otherwise.

Example:

```text
~/.local/state/rhost/
└── jobs/
    └── j_01J.../
        ├── meta.json
        ├── command.sh
        ├── stdout.log
        ├── stderr.log
        ├── pid
        ├── pgid
        ├── exit_code
        └── finished_at
```

Permissions should default to user-private.

Example metadata:

```json
{
  "schema_version": 1,
  "id": "j_01J...",
  "cwd": "/home/dev/work/foo",
  "command": "python train.py",
  "started_at": "2026-09-10T12:00:00Z",
  "backend": "detached"
}
```

Do not store secrets in command metadata if they were passed through environment or future secret mechanisms.

---

## 22. Detached process backend

v0.1 may use:

```text
nohup + setsid + shell wrapper
```

The wrapper should:

1. establish cwd/env;
2. launch the command in its own process group/session;
3. redirect stdout and stderr to distinct files;
4. record PID/PGID;
5. atomically write exit code on completion;
6. atomically write completion time;
7. preserve enough metadata for later rediscovery.

Prefer transferring or generating a small wrapper script over constructing an unreadable deeply quoted one-liner.

### Later backend

If real usage proves a need, add a `systemd --user` job backend.

Do not require systemd for the first usable release.

---

## 23. Job status

`rhost job status` must not infer running state from log freshness.

Use process existence and terminal metadata.

States:

```text
starting
running
exited
failed
stopped
unknown
stale
```

The exact state machine should be documented in code and tests.

If the job metadata says it was running but the PID no longer exists and there is no exit-code file, report `unknown` or `stale`, not `success`.

---

## 24. Job log polling

Support offsets:

```bash
rhost job logs gpu j_01J... --stream stdout --since 8192 --json
```

Return:

```json
{
  "job_id": "j_01J...",
  "stream": "stdout",
  "from": 8192,
  "next": 12288,
  "data": "...",
  "more": false
}
```

This makes polling cheap for agents.

If arbitrary binary output must be supported, expose encoding metadata or base64 when needed.

Codex should inspect Portal's background-job polling implementation for ideas about:

- offset-based incremental reads;
- bounded chunks;
- process discovery after reconnect;
- job metadata lifecycle.

Do not copy its local in-memory registry as the source of truth.

---

## 25. Job signals

Minimum:

```bash
rhost job stop <host> <id>
rhost job kill <host> <id>
```

Prefer signaling the process group rather than one shell PID so children do not remain orphaned.

Behavior should be observable and idempotent.

A stop request against an already terminal job should not invent an error if the requested final condition already holds.

---

