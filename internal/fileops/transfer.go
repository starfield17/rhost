// Package fileops moves files between this machine and an SSH-reachable host.
//
// Two mechanisms, both borrowed from tools a user already has
// (docs/ARCHITECTURE.md §27):
//
//   - scp   — a single file in either direction;
//   - rsync — directory sync, with the incremental behaviour and explicit
//     deletion that §28 needs.
//
// Neither one resolves a host: the target is passed to the tool verbatim, so
// OpenSSH stays the source of truth for the alias, user, port, key and host-key
// policy (AGENTS.md §5). This package builds argument vectors, runs them, and
// parses output; the safety rules are pure functions, so every one of them is
// testable without a machine.
package fileops

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

// Backend names recorded in transfer results, so an agent can tell which tool
// did the work (AGENTS.md §6).
const (
	BackendScp   = "scp"
	BackendRsync = "rsync"
)

// ErrRejected is returned for a request refused before anything runs: a
// dangerous sync destination, an ambiguous path, a missing argument. The app
// layer maps it to SYNC_REJECTED.
type ErrRejected struct{ Msg string }

func (e ErrRejected) Error() string { return e.Msg }

func reject(format string, a ...interface{}) error {
	return ErrRejected{Msg: fmt.Sprintf(format, a...)}
}

// RemoteSpec renders the `host:path` form scp and rsync understand. The host is
// whatever the user named — an alias, user@host, or a bare hostname — and is
// never rewritten.
func RemoteSpec(host, path string) string {
	if strings.Contains(host, ":") && !strings.Contains(host, "[") {
		if i := strings.LastIndex(host, "@"); i >= 0 {
			host = host[:i+1] + "[" + host[i+1:] + "]"
		} else {
			host = "[" + host + "]"
		}
	}
	return host + ":" + path
}

// LocalArg rewrites a local path so scp and rsync cannot mistake it for a remote
// spec. Both tools split on the first colon, and both treat a leading dash as an
// option, so `a:b.txt` and `-p` would otherwise be misread rather than rejected.
//
// The rewrite is only ever a "./" prefix: it names the same file, and it is
// skipped for paths that are already absolute or already explicit.
func LocalArg(path string) (string, error) {
	if path == "" {
		return "", reject("empty local path")
	}
	if strings.ContainsAny(path, "\n\x00") {
		return "", reject("local path %q contains a newline or NUL", path)
	}
	if filepath.IsAbs(path) || path == "." || path == ".." || strings.HasPrefix(path, "./") || strings.HasPrefix(path, "../") {
		// ".", "..", "./x" and any absolute path are already unambiguous.
		return path, nil
	}
	if strings.HasPrefix(path, "-") || strings.Contains(path, ":") {
		return "./" + path, nil
	}
	return path, nil
}

// ValidateTransferPaths checks the arguments of a single-file put or get.
//
// rhost does not try to guess what the remote path "means": it refuses the cases
// where being wrong is destructive or where the tool would misparse the
// argument, and otherwise gets out of the way.
func ValidateTransferPaths(source, destination string) error {
	if strings.TrimSpace(source) == "" {
		return reject("empty source path")
	}
	if strings.ContainsAny(source, "\n\x00") {
		return reject("source path %q contains a newline or NUL", source)
	}
	if _, remote, ok := SplitRemoteSpec(source); ok && strings.ContainsAny(remote, "*?[") {
		return reject("source path %q contains a glob; fs get copies exactly one file", remote)
	}
	return checkDestination(destination)
}

// SplitRemoteSpec recognises the `host:path` form scp and rsync use, with the
// same rule those tools apply: a colon before any slash means a host follows. A
// spec is only ever *parsed* here to validate the path part — the host is passed
// through to OpenSSH untouched, which resolves it (AGENTS.md §5).
func SplitRemoteSpec(arg string) (host, path string, ok bool) {
	i := strings.Index(arg, ":")
	if bracket := strings.IndexByte(arg, '['); bracket >= 0 && (i < 0 || bracket < i) {
		end := strings.IndexByte(arg[bracket:], ']')
		if end < 0 {
			return "", arg, false
		}
		i = bracket + end + 1
		if i >= len(arg) || arg[i] != ':' {
			return "", arg, false
		}
	}
	if i <= 0 || strings.Contains(arg[:i], "/") {
		return "", arg, false
	}
	return arg[:i], arg[i+1:], true
}

// checkDestination rejects the destination shapes that must never reach a remote
// tool: empty, the filesystem root, a glob, or control characters — on either
// side of a remote spec.
func checkDestination(path string) error {
	if host, remote, ok := SplitRemoteSpec(path); ok {
		if err := checkDestination(remote); err != nil {
			return err
		}
		_ = host
		return nil
	}
	if strings.TrimSpace(path) == "" {
		return reject("empty destination path")
	}
	if strings.ContainsAny(path, "\n\x00") {
		return reject("destination path %q contains a newline or NUL", path)
	}
	if strings.ContainsAny(path, "*?[") {
		return reject("destination path %q contains a glob character: the remote shell would expand it", path)
	}
	if trimmed := strings.TrimRight(path, "/"); trimmed == "" {
		return reject("destination path %q is the filesystem root", path)
	}
	return nil
}

// ScpArgs builds the scp argv for one copy in either direction. src and dst must
// already be in final scp form: a local side prepared with LocalArg, a remote one
// with RemoteSpec. Rewriting them here would be wrong — `gpu:~/x` contains a
// colon, and the only thing that knows which side is remote is the caller.
//
// Options are separate argv elements, so scp has no whitespace-splitting problem
// and always reuses the multiplexed connection. -q keeps scp's progress bar off
// stdout, which `--json` must never mix with data (AGENTS.md §6).
func ScpArgs(src, dst string, sshOpts []string) ([]string, error) {
	a, err := transferArg(src)
	if err != nil {
		return nil, err
	}
	b, err := transferArg(dst)
	if err != nil {
		return nil, err
	}
	args := make([]string, 0, len(sshOpts)+3)
	args = append(args, sshOpts...)
	args = append(args, "-q", a, b)
	return args, nil
}

// transferArg refuses an operand that cannot be passed safely rather than
// rewriting it: an empty one, one carrying control characters, or one a tool
// would read as an option.
func transferArg(arg string) (string, error) {
	switch {
	case arg == "":
		return "", reject("empty path")
	case strings.ContainsAny(arg, "\n\x00"):
		return "", reject("path %q contains a newline or NUL", arg)
	case strings.HasPrefix(arg, "-"):
		return "", reject("path %q starts with a dash; prefix it with ./", arg)
	}
	return arg, nil
}

// Result is the outcome of one local tool invocation.
type Result struct {
	Stdout   []byte
	Stderr   []byte
	ExitCode int
	TimedOut bool
	Duration time.Duration
}

// Runner executes scp and rsync. The binary names are fields rather than
// constants so unit tests can prove argument construction against a stub and
// never need the real tools installed.
type Runner struct {
	ScpBin   string
	RsyncBin string
}

// NewRunner returns a Runner that uses the tools on PATH.
func NewRunner() *Runner { return &Runner{ScpBin: "scp", RsyncBin: "rsync"} }

// Run executes one tool with argv and a deadline. A non-nil error means the tool
// could not be started at all (typically: not installed); otherwise the exit
// status is in Result.
func (r *Runner) Run(ctx context.Context, bin string, args []string, timeout time.Duration) (Result, error) {
	if timeout <= 0 {
		timeout = 5 * time.Minute
	}
	cctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	cmd := exec.CommandContext(cctx, bin, args...)
	var stdout, stderr bytes.Buffer
	cmd.Stdout, cmd.Stderr = &stdout, &stderr
	// Closed stdin: scp must never stop to ask for a password in a context where
	// authentication is supposed to come from keys or an agent (AGENTS.md §5).
	cmd.Stdin = strings.NewReader("")
	// A timeout must stop the tool, not merely abandon it: the tool runs in its own
	// process group and cancellation kills the group (see configureProcessGroup),
	// and WaitDelay is the backstop for a child that somehow escapes it and keeps
	// stdout/stderr open.
	configureProcessGroup(cmd)
	cmd.WaitDelay = transferKillGrace

	start := time.Now()
	res := Result{}
	err := cmd.Run()
	res.Duration = time.Since(start)
	res.Stdout, res.Stderr = stdout.Bytes(), stderr.Bytes()

	timedOut := errors.Is(cctx.Err(), context.DeadlineExceeded)
	var ee *exec.ExitError
	switch {
	case err == nil:
		res.ExitCode = 0
	case errors.As(err, &ee):
		res.ExitCode = ee.ExitCode()
	case errors.Is(err, exec.ErrWaitDelay):
		// WaitDelay fired: the process is gone (or the deadline elapsed) but its I/O
		// did not settle. On a real timeout this is still a timeout and is reported
		// below; without one it is not a silent success.
		if !timedOut {
			return res, err
		}
		res.ExitCode = 124
	default:
		return res, err
	}
	if timedOut {
		res.TimedOut = true
		// A killed transfer exits with whatever it was doing; the timeout is the
		// fact that matters, so make it distinguishable from a tool failure.
		if res.ExitCode == 0 {
			res.ExitCode = 124
		}
	}
	return res, nil
}

// transferKillGrace bounds how long Run waits for a killed tool's I/O to settle,
// so a child that escaped the process group cannot hang it indefinitely.
const transferKillGrace = 2 * time.Second
