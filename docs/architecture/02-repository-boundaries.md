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
│   │   ├── status.go
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
│   ├── telemetry/
│   │   ├── linux.go
│   │   ├── nvidia.go
│   │   └── model.go
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
│   └── watch/
│       └── watch.go
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
- `telemetry`

`output` only formats models already produced by the application layer.

Use Go `internal/` aggressively. Do not create a generic `utils/` package.

---

## 4. Suggested core interfaces

Do not over-abstract before the first implementation exists. These interfaces are enough to separate semantics from mechanisms.

Conceptually:

```go
type ExecRequest struct {
    Host    string
    Command string
    Cwd     string
    Env     map[string]string
    Timeout time.Duration
}

type ExecResult struct {
    Host       string
    ExitCode   int
    Stdout     []byte
    Stderr     []byte
    Duration   time.Duration
    TimedOut   bool
}

type Executor interface {
    Exec(ctx context.Context, req ExecRequest) (ExecResult, error)
}
```

Session:

```go
type SessionManager interface {
    Create(ctx context.Context, req CreateSessionRequest) (Session, error)
    List(ctx context.Context, host string) ([]Session, error)
    Exec(ctx context.Context, req SessionExecRequest) (SessionExecResult, error)
    Send(ctx context.Context, req SessionSendRequest) error
    Read(ctx context.Context, req SessionReadRequest) (SessionReadResult, error)
    Close(ctx context.Context, host, id string) error
}
```

Job:

```go
type JobManager interface {
    Start(ctx context.Context, req StartJobRequest) (Job, error)
    List(ctx context.Context, host string) ([]Job, error)
    Status(ctx context.Context, host, id string) (JobStatus, error)
    Logs(ctx context.Context, req JobLogsRequest) (JobLogChunk, error)
    Signal(ctx context.Context, host, id string, sig string) error
}
```

Keep these internal until a second frontend genuinely needs a public Go library.

---
