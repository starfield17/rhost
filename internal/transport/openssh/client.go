package openssh

import (
	"bytes"
	"context"
	"errors"
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
	Stdout   []byte
	Stderr   []byte
	ExitCode int
	TimedOut bool
	Duration time.Duration
}

// options returns the shared OpenSSH options. ControlMaster=auto +
// ControlPersist + a %C ControlPath give us transport reuse that outlives any
// single rhost process, without a daemon (docs/ARCHITECTURE.md §8).
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

// Run executes remoteCmd on target. remoteCmd is passed verbatim as a single
// argument to ssh, so the caller is responsible for producing something the
// remote login shell can parse.
//
// A non-nil error means ssh could not be started at all; otherwise the ssh exit
// status is in Result.ExitCode and a transport-level timeout is in Result.TimedOut.
func (c *Client) Run(ctx context.Context, target, remoteCmd string, timeout time.Duration) (Result, error) {
	_ = config.EnsureControlDir()

	if timeout <= 0 {
		timeout = 60 * time.Second
	}
	cctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	args := append(c.options(), "-T", "--", target, remoteCmd)
	cmd := exec.CommandContext(cctx, c.cfg.SSHBin, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	cmd.Stdin = nil // no stdin: remote commands read EOF

	var res Result
	start := time.Now()
	runErr := cmd.Run()
	res.Duration = time.Since(start)
	res.Stdout = stdout.Bytes()
	res.Stderr = stderr.Bytes()

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
