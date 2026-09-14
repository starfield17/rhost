package app

import (
	"fmt"
	"strings"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// classifyMissingMarker maps a run whose completion marker never appeared onto
// an adapter error. This is where transport failures are distinguished from a
// command that simply exited non-zero.
func classifyMissingMarker(res openssh.Result) *errs.Error {
	stderr := string(res.Stderr)
	lower := strings.ToLower(stderr)

	// The wrapper itself could not start: the remote lacks bash, setsid, or a base64
	// decoder that understands -d (WrapScript carries the script as base64 so that
	// nothing in it has to survive the account's login shell). Quote the offending
	// line, not just the first stderr line, which is often an unrelated banner when
	// ssh is running at a higher log level.
	if strings.Contains(lower, "command not found") &&
		(strings.Contains(lower, "setsid") || strings.Contains(lower, "bash") ||
			strings.Contains(lower, "base64")) {
		return errs.New(errs.RemoteDependencyMissing,
			lineMatching(stderr, "command not found"), false)
	}

	if e := classifySSH(stderr, res.ExitCode); e != nil {
		return e
	}

	if res.ExitCode == 0 {
		return errs.New(errs.Internal,
			"remote command produced no completion marker", false)
	}
	msg := firstLine(stderr)
	if msg == "" {
		// ssh said nothing, which happens for real failures: at LogLevel=ERROR
		// OpenSSH suppresses its own connection diagnostics. Point at the switch
		// that makes the cause visible instead of reporting a bare "no diagnostic".
		return errs.Wrap(errs.RemoteExecutionUnknown,
			fmt.Sprintf("ssh failed with exit status %d and no diagnostic; "+
				"re-run with RHOST_SSH_LOG_LEVEL=VERBOSE to see OpenSSH's reason", res.ExitCode),
			false, nil)
	}
	return errs.Wrap(errs.RemoteExecutionUnknown, msg, false, nil)
}

// classifySSH maps OpenSSH's own stderr diagnostics onto taxonomy codes. This
// never disables host-key checking; it only makes failures machine-readable.
func classifySSH(stderr string, exitCode int) *errs.Error {
	s := stderr
	switch {
	case strings.Contains(s, "ControlPath too long"),
		strings.Contains(s, "unix_listener: cannot bind to path"):
		// rhost keeps its socket short, so this means the environment asked for an
		// unusable path: name the knob instead of reporting a transport fault.
		return errs.Wrap(errs.ConfigInvalid, firstLine(s), false, nil)
	case strings.Contains(s, "Host key verification failed"):
		return errs.New(errs.HostKeyFailed, "host key verification failed", false)
	case exitCode == 255 && (strings.Contains(s, "Permission denied (publickey") ||
		strings.Contains(s, "Permission denied (password") ||
		strings.Contains(s, "Permission denied (keyboard-interactive") ||
		strings.Contains(s, "no supported authentication methods")):
		return errs.Wrap(errs.SSHAuthFailed, firstLine(stderr), false, nil)
	case strings.Contains(s, "Could not resolve hostname"),
		strings.Contains(s, "Name or service not known"):
		return errs.Wrap(errs.HostUnknown, firstLine(stderr), false, nil)
	case strings.Contains(s, "Connection refused"):
		return errs.Wrap(errs.SSHUnreachable, "connection refused", true, nil)
	case strings.Contains(s, "Connection timed out"),
		strings.Contains(s, "Operation timed out"):
		return errs.Wrap(errs.SSHUnreachable, "connection timed out", true, nil)
	case strings.Contains(s, "No route to host"):
		return errs.Wrap(errs.SSHUnreachable, "no route to host", true, nil)
	}
	return nil
}

// lineMatching returns the first line of s containing needle (case-insensitive),
// falling back to the first line when nothing matches.
func lineMatching(s, needle string) string {
	lower := strings.ToLower(needle)
	for _, line := range strings.Split(s, "\n") {
		if strings.Contains(strings.ToLower(line), lower) {
			return strings.TrimSpace(line)
		}
	}
	return firstLine(s)
}

func firstLine(s string) string {
	s = strings.TrimSpace(s)
	if s == "" {
		return ""
	}
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return strings.TrimSpace(s[:i])
	}
	return s
}
