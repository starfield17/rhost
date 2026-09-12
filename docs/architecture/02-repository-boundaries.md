[← Architecture map](../ARCHITECTURE.md)

# Part II — Repository boundaries

## 3. Recommended Go repository layout

Use Go-native layout rather than an artificial `src/` layer:

```text
rhost/
├── README.md
├── skills/
│   └── rhost/               # the installable skill (SKILL.md + references/)
├── docs/
│   ├── PROJECT_OVERVIEW.md
│   ├── ARCHITECTURE.md      # the map
│   └── architecture/        # one file per Part, linked from the map
├── go.mod
├── go.sum
│
├── cmd/
│   └── rhost/
│       └── main.go
│
├── internal/
│   ├── app/
│   │   ├── exec.go
│   │   ├── session.go
│   │   ├── job.go
│   │   ├── fs.go
│   │   └── doctor.go
│   │
│   ├── transport/
│   │   └── openssh/
│   │       ├── client.go
│   │       ├── controlmaster.go
│   │       ├── command.go
│   │       └── config.go
│   │
│   ├── session/
│   │   └── tmux/
│   │       ├── manager.go
│   │       ├── protocol.go
│   │       ├── log.go
│   │       └── attach.go
│   │
│   ├── job/
│   │   └── detached/
│   │       ├── manager.go
│   │       ├── wrapper.go
│   │       ├── metadata.go
│   │       └── logs.go
│   │
│   ├── fileops/
│   │   ├── transfer.go
│   │   └── sync.go
│   │
│   ├── host/
│   │   ├── registry.go
│   │   └── capabilities.go
│   │
│   ├── config/
│   │   ├── config.go
│   │   └── paths.go
│   │
│   ├── output/
│   │   ├── json.go
│   │   └── human.go
│   │
│
├── schemas/
│   └── result-v1.schema.json
│
├── scripts/
│   └── install.sh
│
└── .github/
    └── workflows/
        └── release.yml
```

### Boundary rule

`cmd/rhost` wires dependencies and parses CLI arguments. It contains no remote-control logic.

`internal/app` owns use-case semantics. It depends on interfaces/capabilities, not on terminal formatting.

Backend packages implement mechanisms:

- `transport/openssh`
- `session/tmux`
- `job/detached`
- `fileops`

`output` only formats models already produced by the application layer.

Use Go `internal/` aggressively. Do not create a generic `utils/` package.

---

## 4. Core interfaces

The application layer currently depends directly on the single OpenSSH transport
and concrete persistence backends. Do not introduce an executor, session manager,
or job manager interface until a second implementation actually needs it.

CLI and JSON representations remain outside `internal/app`; remote shell
construction and marker parsing remain under the OpenSSH/session backends. This
keeps a future frontend possible without making it a present abstraction cost.

---
