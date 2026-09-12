package app

import (
	"bytes"
	"context"
	"errors"
	"io"
	"strconv"
	"sync"

	"github.com/starfield17/rhost/internal/config"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/shell"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// StreamExecOptions describes the command-line-shaped foreground path. A zero
// timeout waits until completion or cancellation.
type StreamExecOptions struct {
	ExecOptions
	Stdin  io.Reader
	Stdout io.Writer
	Stderr io.Writer
}

// ExecuteStream runs a foreground command while forwarding its output as it is
// produced. The returned strings contain bounded prefixes for JSON callers; a
// negative MaxOutputBytes disables capture while preserving byte counts.
func (a *App) ExecuteStream(ctx context.Context, opts StreamExecOptions) (ExecResult, *errs.Error) {
	out := ExecResult{Host: opts.Host, ExitCode: -1}
	if e := validateExecOptions(opts.ExecOptions, true); e != nil {
		return out, e
	}
	nonce, err := openssh.NewNonce()
	if err != nil {
		return out, errs.Wrap(errs.Internal, "could not generate nonce", false, err)
	}
	script := openssh.BuildStreamScript(openssh.ExecSpec{Command: opts.Command, Cwd: opts.Cwd, Env: opts.Env, Nonce: nonce})
	runCtx, cancelRun := context.WithCancel(ctx)
	defer cancelRun()
	stdout := newProtocolStream(nonce, opts.Stdout, opts.MaxOutputBytes)
	stderrCap := opts.MaxOutputBytes
	if stderrCap < 0 {
		stderrCap = 64 * 1024
	}
	stderr := newCountedStream(opts.Stderr, stderrCap)
	stdout.cancel, stderr.cancel = cancelRun, cancelRun
	res, runErr := a.SSH.RunPipe(runCtx, opts.Host, openssh.WrapScript(script), openssh.PipeOptions{
		Timeout: opts.Timeout, Stdin: opts.Stdin, Stdout: stdout, Stderr: stderr,
	})
	out.Duration = res.Duration
	out.Stderr, out.StderrBytes, out.StderrTruncated = stderr.result()
	code, complete := stdout.finish()
	out.Stdout, out.StdoutBytes, out.StdoutTruncated = stdout.result()

	if writeErr := firstStreamError(stdout.err(), stderr.err()); writeErr != nil {
		cleanup := a.cleanupExec(opts.Host, nonce)
		out.CleanupConfirmed = cleanup
		return out, errs.Wrap(errs.OutputWriteFailed, "writing command output: "+writeErr.Error(), false, writeErr)
	}
	if res.TimedOut || res.Cancelled {
		out.TimedOut, out.Cancelled = res.TimedOut, res.Cancelled
		out.CleanupConfirmed = a.cleanupExec(opts.Host, nonce)
		if res.Cancelled {
			return out, errs.New(errs.RemoteCommandCancelled, "command cancelled", false)
		}
		if out.CleanupConfirmed {
			return out, executionTimeout(opts.Timeout, true)
		}
		return out, executionTimeout(opts.Timeout, false)
	}
	if runErr != nil {
		if errors.Is(runErr, config.ErrUnsafeLocalState) {
			return out, errs.Wrap(errs.ConfigInvalid, runErr.Error(), false, runErr)
		}
		return out, errs.Wrap(errs.SSHUnreachable, runErr.Error(), true, runErr)
	}
	if !complete {
		res.Stdout = []byte(out.Stdout)
		res.Stderr = []byte(out.Stderr)
		return out, classifyMissingMarker(res)
	}
	out.ExitCode = code
	return out, nil
}

func validateExecOptions(opts ExecOptions, allowNoCapture bool) *errs.Error {
	min := 0
	if allowNoCapture {
		min = -1
	}
	if opts.MaxOutputBytes < min || opts.MaxOutputBytes > maxExecOutputBytes {
		return errs.New(errs.ConfigInvalid, "max output bytes must be between 0 and 67108864", false)
	}
	for k := range opts.Env {
		if err := shell.ValidateEnvKey(k); err != nil {
			return errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
		}
	}
	if len(bytes.TrimSpace([]byte(opts.Command))) == 0 {
		return errs.New(errs.ConfigInvalid, "no command given", false)
	}
	return nil
}

func (a *App) cleanupExec(host, nonce string) bool {
	kctx, cancel := context.WithTimeout(context.Background(), killTimeout)
	defer cancel()
	res, err := a.SSH.Run(kctx, host, openssh.WrapScript(openssh.KillCommand(nonce)), killTimeout)
	return err == nil && !res.TimedOut && bytes.Contains(res.Stdout, []byte("killed:"))
}

type countedStream struct {
	mu       sync.Mutex
	dst      io.Writer
	cap      int
	buf      bytes.Buffer
	total    int64
	writeErr error
	cancel   context.CancelFunc
}

func newCountedStream(dst io.Writer, cap int) *countedStream {
	return &countedStream{dst: dst, cap: cap}
}

func (w *countedStream) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.total += int64(len(p))
	if w.cap >= 0 && (w.cap == 0 || w.buf.Len() < w.cap) {
		n := len(p)
		if w.cap > 0 && n > w.cap-w.buf.Len() {
			n = w.cap - w.buf.Len()
		}
		w.buf.Write(p[:n])
	}
	if w.dst != nil && w.writeErr == nil {
		_, w.writeErr = w.dst.Write(p)
		if w.writeErr != nil {
			if w.cancel != nil {
				w.cancel()
			}
			return len(p), w.writeErr
		}
	}
	return len(p), nil
}

func (w *countedStream) result() (string, int64, bool) {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.buf.String(), w.total, w.cap > 0 && w.total > int64(w.cap)
}
func (w *countedStream) err() error { w.mu.Lock(); defer w.mu.Unlock(); return w.writeErr }

type protocolStream struct {
	*countedStream
	nonce    string
	pending  []byte
	started  bool
	complete bool
	exitCode int
}

func newProtocolStream(nonce string, dst io.Writer, cap int) *protocolStream {
	return &protocolStream{countedStream: newCountedStream(dst, cap), nonce: nonce, exitCode: -1}
}

func (w *protocolStream) Write(p []byte) (int, error) {
	w.pending = append(w.pending, p...)
	begin := []byte("\x00__RHOST_BEGIN_" + w.nonce + "__\n")
	if !w.started {
		if i := bytes.Index(w.pending, begin); i >= 0 {
			w.pending = w.pending[i+len(begin):]
			w.started = true
		} else {
			keep := len(begin) - 1
			if len(w.pending) > keep {
				w.pending = append([]byte(nil), w.pending[len(w.pending)-keep:]...)
			}
			return len(p), nil
		}
	}
	done := []byte("\x00__RHOST_DONE_" + w.nonce + "__:")
	if i := bytes.Index(w.pending, done); i >= 0 {
		if i > 0 {
			if _, err := w.countedStream.Write(w.pending[:i]); err != nil {
				return len(p), err
			}
			w.pending = append([]byte(nil), w.pending[i:]...)
		}
		return len(p), nil
	}
	keep := markerPrefixSuffix(w.pending, done)
	flush := len(w.pending) - keep
	if flush > 0 {
		_, err := w.countedStream.Write(w.pending[:flush])
		w.pending = append([]byte(nil), w.pending[flush:]...)
		if err != nil {
			return len(p), err
		}
	}
	return len(p), nil
}

func markerPrefixSuffix(data, marker []byte) int {
	max := len(data)
	if len(marker)-1 < max {
		max = len(marker) - 1
	}
	for n := max; n > 0; n-- {
		if bytes.Equal(data[len(data)-n:], marker[:n]) {
			return n
		}
	}
	return 0
}

func (w *protocolStream) finish() (int, bool) {
	needle := []byte("\x00__RHOST_DONE_" + w.nonce + "__:")
	i := bytes.LastIndex(w.pending, needle)
	if i < 0 {
		if w.started {
			_, _ = w.countedStream.Write(w.pending)
		}
		w.pending = nil
		return -1, false
	}
	rest := w.pending[i+len(needle):]
	j := bytes.IndexByte(rest, '\n')
	if j < 0 {
		_, _ = w.countedStream.Write(w.pending)
		w.pending = nil
		return -1, false
	}
	code, err := strconv.Atoi(string(rest[:j]))
	if err != nil {
		_, _ = w.countedStream.Write(w.pending)
		w.pending = nil
		return -1, false
	}
	_, _ = w.countedStream.Write(w.pending[:i])
	w.pending = nil
	w.complete, w.exitCode = true, code
	return code, true
}

func firstStreamError(errs ...error) error {
	for _, err := range errs {
		if err != nil {
			return err
		}
	}
	return nil
}
