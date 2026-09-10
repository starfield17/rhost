[← Architecture map](../ARCHITECTURE.md)

# Part XVIII — anti-goals

## 52. Things Codex should actively avoid

Do not:

- create `gpu-test`, `yolo-train`, or project-specific commands;
- make every remote action use a persistent shell;
- run long work as a blocking `exec`;
- store persistent state only in Go maps;
- implement an MCP server first;
- require a daemon for v0.1;
- duplicate `~/.ssh/config`;
- turn off host-key checking to make tests easier;
- auto-install tmux/rsync remotely;
- parse human-formatted output when structured data can exist;
- let `watch` become the owner of jobs/sessions;
- invent a broad plugin framework before a second backend exists;
- introduce interfaces only to make the code "clean";
- create a `utils` dumping ground;
- mix file-sync deletion into a default-safe command;
- hide unknown job/session states as success.

---

