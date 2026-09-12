package openssh

import (
	"bytes"
	"context"
	"errors"
	"io"
	"os"
	"os/exec"
	"strconv"
	"time"

	"github.com/starfield17/rhost/internal/config"
)

// Config controls how the OpenSSH client is invoked.
type Config struct {
	SSHBin         string
	ControlPath    string
	ControlPersist string
	ConnectTimeout time.Duration
	BatchMode      bool
	LogLevel       string
}

// DefaultConfig returns rhost's default OpenSSH invocation settings.
func DefaultConfig() Config {
	return Config{
		SSHBin:         "ssh",
		ControlPath:    config.ControlPath(),
		ControlPersist: "15m",
		ConnectTimeout: 15 * time.Second,
		BatchMode:      true,
		LogLevel:       defaultLogLevel(),
	}
}

// defaultLogLevel keeps ssh quiet on success but lets a caller raise verbosity
// when a transport failure needs diagnosing: OpenSSH suppresses its own
// "Connection closed by ..." style diagnostics at LogLevel=ERROR, which would
// otherwise leave rhost with an empty stderr to classify.
func defaultLogLevel() string {
	if v := os.Getenv("RHOST_SSH_LOG_LEVEL"); v != "" {
		return v
	}
	return "ERROR"
}

// Client runs commands through the system OpenSSH client.
type Client struct {
	cfg Config
}

// New returns a Client, filling in defaults for empty fields.
func New(cfg Config) *Client {
	if cfg.SSHBin == "" {
		cfg.SSHBin = "ssh"
	}
	if cfg.ControlPath == "" {
		cfg.ControlPath = config.ControlPath()
	}
	if cfg.ControlPersist == "" {
		cfg.ControlPersist = "15m"
	}
	if cfg.ConnectTimeout <= 0 {
		cfg.ConnectTimeout = 15 * time.Second
	}
	if cfg.LogLevel == "" {
		cfg.LogLevel = "ERROR"
	}
	return &Client{cfg: cfg}
}

// Config returns the resolved configuration.
func (c *Client) Config() Config { return c.cfg }

// Result is the raw outcome of one ssh invocation.
type Result struct {
	Stdout    []byte
	Stderr    []byte
	ExitCode  int
	TimedOut  bool
	Cancelled bool
	Duration  time.Duration
	// StdoutBytes and StderrBytes are what the remote produced, which can exceed
	// the length of Stdout and Stderr when a cap was asked for.
	StdoutBytes int64
	StderrBytes int64
}

// PipeOptions runs ssh with caller-owned streams. A zero timeout means no
// execution deadline; cancellation still comes from ctx.
type PipeOptions struct {
	Timeout time.Duration
	Stdin   io.Reader
	Stdout  io.Writer
	Stderr  io.Writer
}

// options returns the shared OpenSSH options. ControlMaster=auto +
// ControlPersist + a %C ControlPath give us transport reuse that outlives any
// single rhost process, without a daemon (docs/architecture/runtime.md).
// SSHOptions returns the OpenSSH options this client passes to ssh, in argv form.
//
// scp takes them directly; rsync can only carry them inside a single -e string,
// which is why fileops.SyncArgs needs them and reports whether they could travel.
// No caller may build its own ssh argument list from scratch: rhost's transport
// reuse depends on these options being the only ones (AGENTS.md §5).
func (c *Client) SSHOptions() []string { return c.options() }

func (c *Client) options() []string {
	opts := []string{
		"-o", "ControlMaster=auto",
		"-o", "ControlPersist=" + c.cfg.ControlPersist,
		"-o", "ControlPath=" + c.cfg.ControlPath,
		"-o", "LogLevel=" + c.cfg.LogLevel,
		"-o", "ConnectTimeout=" + strconv.Itoa(int(c.cfg.ConnectTimeout.Seconds())),
	}
	if c.cfg.BatchMode {
		// Never prompt: agents run non-interactively. Authentication must come
		// from keys / ssh-agent.
		opts = append(opts, "-o", "BatchMode=yes")
	}
	return opts
}

// RunOptions tunes one ssh invocation. The zero value is exactly what Run does.
type RunOptions struct {
	// Timeout bounds the ssh process. A value <= 0 uses the 60s default.
	Timeout time.Duration
	// MaxOutputBytes bounds each stream in memory and in Result; 0 is unbounded.
	// The remote command is unaffected: this is about what rhost carries back.
	MaxOutputBytes int
	// Stdin is written to ssh's stdin, so the remote command can read it. Most
	// rhost commands leave it nil, which gives the remote process EOF at once.
	Stdin []byte
}

// Run executes remoteCmd on target. remoteCmd is passed verbatim as a single
// argument to ssh, so the caller is responsible for producing something the
// remote login shell can parse.
//
// A non-nil error means ssh could not be started at all; otherwise the ssh exit
// status is in Result.ExitCode and a transport-level timeout is in Result.TimedOut.
func (c *Client) Run(ctx context.Context, target, remoteCmd string, timeout time.Duration) (Result, error) {
	return c.RunWith(ctx, target, remoteCmd, RunOptions{Timeout: timeout})
}

// MasterAlive asks OpenSSH whether the shared ControlMaster for target is
// actually answering. It never creates a master; false means callers must not
// claim their transfer used a persistent connection merely because the argv
// carried a ControlPath option.
func (c *Client) MasterAlive(ctx context.Context, target string) bool {
	if err := config.EnsureControlDir(); err != nil {
		return false
	}
	check, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	args := append(c.options(), "-O", "check", "--", target)
	return exec.CommandContext(check, c.cfg.SSHBin, args...).Run() == nil
}

// RunWith is Run with an output budget and an optional stdin payload.
func (c *Client) RunWith(ctx context.Context, target, remoteCmd string, opts RunOptions) (Result, error) {
	if err := config.EnsureControlDir(); err != nil {
		return Result{}, err
	}

	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = 60 * time.Second
	}
	cctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	args := append(c.options(), "-T", "--", target, remoteCmd)
	cmd := exec.CommandContext(cctx, c.cfg.SSHBin, args...)
	// A cap is honoured as "keep the first N bytes *and* the last few", so the
	// caller's own cut to N lands inside the prefix rather than gluing the tail of
	// the output onto its head. That is why the capture keeps tailKeep extra.
	keep := opts.MaxOutputBytes
	if keep > 0 {
		keep += tailKeep
	}
	stdout, stderr := capture{limit: keep}, capture{limit: keep}
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	if opts.Stdin != nil {
		cmd.Stdin = bytes.NewReader(opts.Stdin)
	} else {
		cmd.Stdin = nil // no stdin: remote commands read EOF
	}
	// WaitDelay is the backstop when the remote is gone but a child still holds the
	// pipes: without it, cmd.Run could wait past the deadline for a process ssh
	// has already stopped talking to.
	cmd.WaitDelay = 2 * time.Second

	var res Result
	start := time.Now()
	runErr := cmd.Run()
	res.Duration = time.Since(start)
	res.Stdout = stdout.Bytes()
	res.Stderr = stderr.Bytes()
	res.StdoutBytes, res.StderrBytes = stdout.total, stderr.total

	if cctx.Err() == context.DeadlineExceeded {
		res.TimedOut = true
		return res, nil
	}
	if runErr != nil {
		var ee *exec.ExitError
		if errors.As(runErr, &ee) {
			res.ExitCode = ee.ExitCode()
			return res, nil
		}
		return res, runErr
	}
	res.ExitCode = 0
	return res, nil
}

// RunPipe executes one SSH command without buffering its streams in the
// transport. It is used by the command-line-shaped execution path, where bytes
// must be observable before the remote process exits.
func (c *Client) RunPipe(ctx context.Context, target, remoteCmd string, opts PipeOptions) (Result, error) {
	if err := config.EnsureControlDir(); err != nil {
		return Result{}, err
	}
	cctx := ctx
	cancel := func() {}
	if opts.Timeout > 0 {
		cctx, cancel = context.WithTimeout(ctx, opts.Timeout)
	}
	defer cancel()

	args := append(c.options(), "-T", "--", target, remoteCmd)
	cmd := exec.CommandContext(cctx, c.cfg.SSHBin, args...)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = opts.Stdin, opts.Stdout, opts.Stderr
	cmd.WaitDelay = 2 * time.Second

	var res Result
	start := time.Now()
	runErr := cmd.Run()
	res.Duration = time.Since(start)
	if cctx.Err() == context.DeadlineExceeded {
		res.TimedOut = true
		return res, nil
	}
	if ctx.Err() != nil {
		res.Cancelled = true
		return res, nil
	}
	if runErr != nil {
		var ee *exec.ExitError
		if errors.As(runErr, &ee) {
			res.ExitCode = ee.ExitCode()
			return res, nil
		}
		return res, runErr
	}
	return res, nil
}
