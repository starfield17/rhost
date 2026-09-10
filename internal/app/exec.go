package app

import (
	"bytes"
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/shell"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// killTimeout bounds the best-effort remote process-group kill issued after a
// foreground timeout.
const killTimeout = 8 * time.Second

// ExecOptions describes a single stateless foreground execution.
type ExecOptions struct {
	Host    string
	Command string
	Cwd     string
	Env     map[string]string
	Timeout time.Duration
}

// ExecResult is the outcome of a foreground execution.
type ExecResult struct {
	Host     string
	ExitCode int
	Stdout   string
	Stderr   string
	Duration time.Duration
	TimedOut bool
}

// Execute runs a command in a fresh remote execution context.
//
// A non-nil *errs.Error indicates an adapter/transport failure (or a timeout).
// A nil error means the remote command actually ran; ExitCode is then the
// remote command's real status.
func (a *App) Execute(ctx context.Context, opts ExecOptions) (ExecResult, *errs.Error) {
	out := ExecResult{Host: opts.Host, ExitCode: -1}

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

	res, runErr := a.SSH.Run(ctx, opts.Host, openssh.WrapScript(script), timeout)
	if runErr != nil {
		return out, errs.Wrap(errs.SSHUnreachable, runErr.Error(), true, runErr)
	}
	out.Duration = res.Duration

	if res.TimedOut {
		out.Stdout = string(res.Stdout)
		out.Stderr = string(res.Stderr)

		// Killing the local ssh process does NOT terminate the remote command
		// (verified on a real host), so explicitly kill the recorded process
		// group. The recorded PID also tells us whether the command ever
		// started: if there is no PID file, the host never became reachable and
		// this was a connection hang, not a slow command.
		kctx, cancel := context.WithTimeout(context.Background(), killTimeout)
		kres, kerr := a.SSH.Run(kctx, opts.Host, openssh.WrapScript(openssh.KillCommand(nonce)), killTimeout)
		cancel()

		if kerr == nil && !kres.TimedOut && bytes.Contains(kres.Stdout, []byte("killed:")) {
			out.TimedOut = true
			return out, errs.New(errs.RemoteCommandTimeout,
				fmt.Sprintf("command exceeded timeout %s", timeout), true)
		}
		return out, errs.New(errs.SSHUnreachable,
			"host did not become reachable before the timeout; the command did not start", true)
	}

	body, code, ok := openssh.ParseMarker(res.Stdout, nonce)
	if !ok {
		return out, classifyMissingMarker(res)
	}

	out.ExitCode = code
	out.Stdout = string(body)
	out.Stderr = string(res.Stderr)
	return out, nil
}
