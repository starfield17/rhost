package app

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/config"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/shell"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// killTimeout bounds the best-effort remote process-group kill issued after a
// foreground timeout.
const killTimeout = 8 * time.Second

// maxExecOutputBytes keeps one invocation's two in-memory streams bounded even
// when an API caller bypasses the CLI's flag validation.
const maxExecOutputBytes = 64 * 1024 * 1024

// ExecOptions describes a single stateless foreground execution.
type ExecOptions struct {
	Host    string
	Command string
	Cwd     string
	Env     map[string]string
	Timeout time.Duration
	// Stdin is fed to the remote command. It exists for the helpers that take a
	// structured request rather than a path, so no argument ever has to be quoted
	// into a shell (fs read/write/patch use it).
	Stdin []byte
	// MaxOutputBytes bounds each output stream *in the CLI's memory and in the
	// JSON*. 0 means unbounded, which is what `exec` used to do always; the
	// command itself is never truncated, only what rhost carries back.
	MaxOutputBytes int
}

// ExecResult is the outcome of a foreground execution.
//
// The `*_bytes` counters are what the command produced, the `*_truncated` flags
// say whether `Stdout`/`Stderr` are all of it, and `CleanupConfirmed` separates
// "this run is over" from "this run is over and I proved the remote stopped".
type ExecResult struct {
	Host         string
	ExitCode     int
	Stdout       string
	Stderr       string
	Duration     time.Duration
	TimedOut     bool
	Cancelled    bool
	CancelSignal string

	StdoutBytes      int64
	StderrBytes      int64
	StdoutTruncated  bool
	StderrTruncated  bool
	CleanupConfirmed bool
}

// Execute runs a command in a fresh remote execution context.
//
// A non-nil *errs.Error indicates an adapter/transport failure (or a timeout).
// A nil error means the remote command actually ran; ExitCode is then the
// remote command's real status.
func (a *App) Execute(ctx context.Context, opts ExecOptions) (ExecResult, *errs.Error) {
	out := ExecResult{Host: opts.Host, ExitCode: -1}
	if opts.MaxOutputBytes < 0 || opts.MaxOutputBytes > maxExecOutputBytes {
		return out, errs.New(errs.ConfigInvalid,
			"max output bytes must be between 0 and 67108864", false)
	}

	for k := range opts.Env {
		if err := shell.ValidateEnvKey(k); err != nil {
			return out, errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
		}
	}
	if strings.TrimSpace(opts.Command) == "" {
		return out, errs.New(errs.ConfigInvalid, "no command given", false)
	}

	nonce, err := openssh.NewNonce()
	if err != nil {
		return out, errs.Wrap(errs.Internal, "could not generate nonce", false, err)
	}

	script := openssh.BuildScript(openssh.ExecSpec{
		Command: opts.Command,
		Cwd:     opts.Cwd,
		Env:     opts.Env,
		Nonce:   nonce,
	})

	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = a.DefaultTimeout
	}

	res, runErr := a.SSH.RunWith(ctx, opts.Host, openssh.WrapScript(script), openssh.RunOptions{
		Timeout:        timeout,
		MaxOutputBytes: opts.MaxOutputBytes,
		Stdin:          opts.Stdin,
	})
	if runErr != nil {
		if errors.Is(runErr, config.ErrUnsafeLocalState) {
			return out, errs.Wrap(errs.ConfigInvalid, runErr.Error(), false, runErr)
		}
		return out, errs.Wrap(errs.SSHUnreachable, runErr.Error(), true, runErr)
	}
	out.Duration = res.Duration

	if res.TimedOut {
		out.TimedOut = true
		out.Stdout = string(res.Stdout)
		out.Stderr = string(res.Stderr)
		out.StdoutBytes, out.StderrBytes = res.StdoutBytes, res.StderrBytes
		limitExecOutput(&out, opts.MaxOutputBytes)

		// Killing the local ssh process does NOT terminate the remote command
		// (verified on a real host), so explicitly kill the recorded process
		// group. What that second call cannot prove is the important part: a
		// missing pid file or a failed cleanup is equally consistent with the
		// command still running, so rhost reports the uncertainty instead of
		// claiming the command never started.
		kctx, cancel := context.WithTimeout(context.Background(), killTimeout)
		kres, kerr := a.SSH.Run(kctx, opts.Host, openssh.WrapScript(openssh.KillCommand(nonce)), killTimeout)
		cancel()

		if kerr == nil && !kres.TimedOut && bytes.Contains(kres.Stdout, []byte("killed:")) {
			out.CleanupConfirmed = true
			return out, errs.New(errs.RemoteCommandTimeout,
				fmt.Sprintf("command exceeded timeout %s", timeout), true)
		}
		return out, errs.New(errs.SSHUnreachable,
			"execution deadline exceeded; remote cleanup could not be confirmed, "+
				"so the command may still be running", true)
	}

	body, code, ok := openssh.ParseMarker(res.Stdout, nonce)
	if !ok {
		out.Stdout, out.Stderr = string(res.Stdout), string(res.Stderr)
		out.StdoutBytes, out.StderrBytes = res.StdoutBytes, res.StderrBytes
		limitExecOutput(&out, opts.MaxOutputBytes)
		return out, classifyMissingMarker(res)
	}

	out.ExitCode = code
	out.Stdout = string(body)
	out.Stderr = string(res.Stderr)
	// The counters are of everything the wrapper printed, which includes the begin
	// and completion markers; the body does not. Subtracting what ParseMarker
	// removed keeps stdout_bytes the size of the *command's* output, so a truncated
	// stream is not reported as slightly larger than it was.
	out.StdoutBytes = res.StdoutBytes - int64(len(res.Stdout)-len(body))
	out.StderrBytes = res.StderrBytes
	limitExecOutput(&out, opts.MaxOutputBytes)
	return out, nil
}

// limitExecOutput cuts each stream to the caller's budget, on a rune boundary.
//
// The transport already kept a bounded prefix plus a protocol-sized suffix, so
// the completion marker survives a cap and the exit code is still the command's
// own; what this does is make the *reported* stream obey the limit that was
// asked for, and set the truncation flags from the true byte counts.
func limitExecOutput(out *ExecResult, limit int) {
	if limit <= 0 {
		return
	}
	cut := func(s string) string {
		if len(s) <= limit {
			return s
		}
		b := []byte(s[:limit])
		return string(b[:len(b)-shell.IncompleteUTF8Suffix(b)])
	}
	out.Stdout, out.Stderr = cut(out.Stdout), cut(out.Stderr)
	out.StdoutTruncated = out.StdoutBytes > int64(limit)
	out.StderrTruncated = out.StderrBytes > int64(limit)
}
