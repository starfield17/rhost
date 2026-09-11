[← Architecture map](../ARCHITECTURE.md)

# Part VIII — files

## 26. File operations

Target surface:

```text
rhost fs put
rhost fs get
rhost fs sync
```

Examples:

```bash
rhost fs put gpu ./model.py ~/work/foo/model.py
rhost fs get gpu ~/work/foo/results.json ./results.json
rhost fs sync gpu ./project ~/work/project
```

`fs` operates on generic paths. It knows nothing about repositories, datasets, checkpoints, or YOLO.

---

## 27. v0.1 transfer backend

A pragmatic first release may use:

- `rsync` for directory synchronization when available;
- `scp` or rsync for simple file transfer.

Advantages:

- mature behavior;
- incremental sync;
- proven SSH integration;
- uses the same OpenSSH configuration.

Later, native SFTP can remove dependencies or provide tighter structured progress.

Codex may study Portal's SFTP/file-transfer code for design ideas, especially structured errors and transfer integrity, but native SFTP is not required to validate the overall architecture.

---

## 28. Safe synchronization

`sync` is the operation most likely to cause accidental data loss.

Requirements:

- no implicit deletion by default;
- `--delete` must be explicit;
- provide `--dry-run`;
- print/return the effective source and destination;
- reject obviously dangerous empty/root destinations where possible;
- keep agent JSON and human preview consistent;
- do not silently follow unexpected symlinks across boundaries.

Single-file transfers reject a remote source glob: `fs get` copies exactly one
file, never an expansion whose cardinality depends on remote directory contents.
`source` and `destination` name the effective files; when a put target is an
existing remote directory, the destination includes the source basename inside
that directory. `multiplexed` is true only after OpenSSH's control command
observes the shared master answering; carrying a `ControlPath` option alone is
not evidence of reuse.

Recommended agent workflow:

```text
dry run
→ inspect plan
→ execute
```

The Skill should teach this for destructive sync modes.

---
