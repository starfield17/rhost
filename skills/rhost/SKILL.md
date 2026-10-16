---
name: rhost
description: Run commands and perform explicit file, session, tunnel, or diagnostic operations on SSH-reachable hosts through an installed rhost CLI. Use when work belongs on a remote host and OpenSSH should retain authentication and host-key ownership.
---

# rhost

Use an SSH target: alias, `user@example-host`, or hostname (`rhost hosts --json` lists known aliases).
On first use of a host, run `rhost doctor <host> --json` (`python3:true` reports interpreter presence).

## Fast decision tree

| What you want to do | Canonical invocation |
| --- | --- |
| Run command / script | `rhost exec <host> --command '<cmd>'` or `--command-file script.sh` |
| Read file & get hash | `rhost fs read <host> '<path>'` (add `--json` for `data.sha256`) |
| Exact text edit (CAS) | `rhost fs patch <host> '<path>' --json << 'EOF'` with `old`/`new` edits |
| Write / overwrite file | `rhost fs write <host> '<path>' [--if-hash <sha>] --json << 'EOF'` |
| Directory sync | `rhost fs sync <host> ./local '<remote-dir>' --dry-run --json` |
| Persistent shell / session | `rhost session create <host> --name debug` → `exec` / `read` / `send` |
| Background job (zero-daemon)| `rhost exec <host> --command 'nohup <cmd> > job.log 2>&1 </dev/null & echo $! > job.pid'` |
| Port forwarding (tunnel) | `rhost tunnel open <host> --kind local --listen localhost:8080 --destination localhost:8000 --json` |
| Probe & connection reuse | `rhost doctor <host> --json` / `rhost connection status <host> --json` |

## Output mode

- **Read-only exploration**: Use default text mode (no `--json`). Streams stdout/stderr directly with minimal token overhead.
- **Mutations & programmatic branching**: Always use `--json`. Returns a schema-v2 envelope on stdout. Branch on `error.code`, never on English text. `--json --stream` also mirrors output to stderr.

## Remote file editing (CAS workflow)

Editing uses compare-and-swap (CAS) to prevent lost updates:
1. `rhost fs read <host> '<path>' --json` → get current `data.sha256`.
2. Apply changes:
   - **Exact text replacement** (recommended):
     ```bash
     rhost fs patch <host> '<path>' --json << 'EOF'
     {
       "sha256": "<sha256-from-read>",
       "edits": [{"old": "exact old text\n", "new": "new text\n"}]
     }
     EOF
     ```
     `old` must appear exactly once in the file. Line-range edits (`start`, `end`, `text`) are also supported.
   - **Whole file rewrite**:
     ```bash
     rhost fs write <host> '<path>' --if-hash <sha256-from-read> --json << 'EOF'
     new file contents
     EOF
     ```
3. `fs write` and `fs patch` return the new `data.sha256` in the response, allowing chained edits without re-reading. Both read from stdin by default (`--from -` and `--patch -`).

## Execution & Sessions

- **Exec**: Runs via remote `bash -lc` under an OpenSSH ControlMaster (15-minute persistence). Sudo credentials do not persist across separate calls; use `--command-file` or compound commands for multi-step tasks.
- **Background work**: Detach standard Unix processes (`nohup <cmd> > log 2>&1 </dev/null & echo $! > pid`). Check liveness via `kill -0 $(cat pid) 2>/dev/null && echo RUNNING || echo STOPPED`.
- **Sessions**: Use for stateful shells, REPLs, or long-running commands. Drive via `rhost session exec <host> <name> --command '<cmd>'` (or `--command-file`), inspect output with `rhost session read <host> <name> [--tail <bytes>] [--since <offset>]`, and send raw keys via `send`. `session attach` is refused.

## Authority & References

- Read [references/CLI.md](references/CLI.md) for complete flag documentation, batch manifests, and advanced examples.
- Read [references/RECOVERY.md](references/RECOVERY.md) for `error.code` error classification and recovery procedures.
- Read [references/SAFETY.md](references/SAFETY.md) for destructive operations, sudo, and security invariants.
- If flags or behavior seem unexpected, verify the installed binary with `rhost version --json` and `rhost <command> --help` (symlinked skill documentation can drift from the binary).
