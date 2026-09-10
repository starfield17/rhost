package app

import (
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

	// The wrapper itself could not start: the remote lacks bash or setsid.
	if strings.Contains(lower, "command not found") &&
		(strings.Contains(lower, "setsid") || strings.Contains(lower, "bash")) {
		return errs.New(errs.RemoteDependencyMissing, firstLine(stderr), false)
	}

	if e := classifySSH(stderr); e != nil {
		return e
	}

	if res.ExitCode == 0 {
		return errs.New(errs.Internal,
			"remote command produced no completion marker", false)
	}
	msg := firstLine(stderr)
	if msg == "" {
		msg = "ssh failed with no diagnostic"
	}
	return errs.Wrap(errs.SSHUnreachable, msg, true, nil)
}

// classifySSH maps OpenSSH's own stderr diagnostics onto taxonomy codes. This
// never disables host-key checking; it only makes failures machine-readable.
func classifySSH(stderr string) *errs.Error {
	s := stderr
	switch {
	case strings.Contains(s, "Host key verification failed"):
		return errs.New(errs.HostKeyFailed, "host key verification failed", false)
	case strings.Contains(s, "Permission denied"),
		strings.Contains(s, "no supported authentication methods"):
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
	case strings.Contains(s, "Connection closed"),
		strings.Contains(s, "Connection reset"),
		strings.Contains(s, "Broken pipe"):
		return errs.Wrap(errs.SSHUnreachable, firstLine(stderr), true, nil)
	}
	return nil
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
