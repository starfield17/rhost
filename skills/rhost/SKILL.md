---
name: rhost
description: Run commands, persistent sessions, detached jobs, and file operations on an SSH-reachable remote host with the rhost CLI. Use when work must execute on a remote machine (GPU box, build server, test host, Linux node) instead of this machine.
---

# rhost — remote host adapter

`rhost` treats an SSH-reachable machine as a reusable execution node. It
orchestrates your existing OpenSSH configuration and never duplicates SSH
authentication or host-key policy. There is no server to run and no state on this
machine that the remote side depends on: sessions live in remote tmux, jobs in
detached remote processes.

Use it when the work belongs on the remote host. For work that is local to this
machine, use the ordinary tools.

**Always pass `--json`, and branch on `error.code`.** Never parse human output or
English messages when a JSON form exists.

## Rules that always hold

- **Probe before relying.** `rhost doctor <host> --json` once per host. Do not
  assume `tmux`, `rsync`, `python3`, `rg`, or anything else exists: a missing
  dependency is `REMOTE_DEPENDENCY_MISSING`, and rhost never installs one.
- **Never put a secret on a command line.** Command text is written to the local
  audit log and visible to `ps` on the remote host. Use the environment or a file
  that already holds it.
- **Never weaken SSH to make something work.** Host-key checking stays on, keys
  and passwords are not persisted, and `HOST_KEY_FAILED` means investigate.
- **After a disconnect, re-query; never assume.** `session list` and `job list`
  are the truth, not what this process remembers.
- **Preview destructive file operations.** `fs sync --dry-run` first; `--delete`
  prunes remote-only files and is refused for whole-home or top-level targets.
- **Respect the exit-code policy.** A status in 0–254 is the remote command's
  own; 124 is a foreground timeout (`REMOTE_COMMAND_TIMEOUT`); 255 is an adapter
  failure, whose reason is `error.code`. When a remote command can itself exit
  124 or 255, read the JSON envelope instead of inferring from the status.

## Choose the tool

| need | use |
|---|---|
| one command, no state to keep | `rhost exec` |
| cwd, env, or a REPL must persist across calls | `rhost session` |
| the work must outlive the connection | `rhost job` |
| move, read, or edit files | `rhost fs` |
| what is happening on the host | `rhost status`; `rhost watch` is for humans |
| one command on several hosts | `rhost exec-many` |
| a port forward that outlives this call | `rhost tunnel` |

Read [references/CLI.md](references/CLI.md) for the commands and their flags,
[references/RECOVERY.md](references/RECOVERY.md) when something failed, and
[references/SAFETY.md](references/SAFETY.md) before a destructive, exposing, or
secret-touching operation.

## The three answers that decide the next step

- `exec` is **stateless**: each call is a fresh login shell. Pass `--cwd`,
  `--env`, `--timeout` explicitly.
- `session exec` runs a command in the managed shell, and **refuses with
  `SESSION_BUSY`** when a program — a REPL, a debugger, an editor — owns the
  pane. That is not a broken session: drive the program with `session send` and
  `session read`, or interrupt it with `session recover`. A refused command never
  ran, so nothing has to be undone.
- A job is `running` only while its process identity is verified. `stale` means
  the recorded process is gone **or** its pid now belongs to another process:
  never a success, and `job stop`/`job kill` report `signalled: false` when they
  refuse to signal a process group they cannot tie to the job. `data.exit_code`
  is the result that counts; a job terminated by its own stop records `143`.

## Capabilities

`hosts`, `doctor`, `exec`, `exec-many`, `session`
(create/list/exec/send/read/close/attach/recover), `job`
(start/list/status/logs/stop/kill), `fs`
(put/get/sync/mirror/batch/read/write/patch/grep/glob), `tunnel`
(open/list/close), `status`, `watch`, `audit`, `version`.

If a command is not in that list, it does not exist yet. `rhost <command> --help`
is authoritative for the build in front of you; never invent a flag.
