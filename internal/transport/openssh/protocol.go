// Package openssh implements rhost's v0.1 transport: it orchestrates the system
// OpenSSH client rather than reimplementing the SSH protocol. OpenSSH stays the
// source of truth for authentication, host-key policy, ProxyJump, and
// connection reuse (ControlMaster).
//
// This file (protocol.go) defines the *remote execution protocol*: a wrapper
// script that runs a command in its own session/process group, records its PGID
// so a timed-out command can be killed remotely, and prints a random completion
// marker carrying the real exit status.
package openssh

import (
	"bytes"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"sort"
	"strconv"
	"strings"

	"github.com/starfield17/rhost/internal/shell"
)

const (
	markerPrefix   = "__RHOST_DONE_"
	outBeginPrefix = "__RHOST_BEGIN_"

	// DefaultRemoteStateDir is the remote per-user state root. It is a shell
	// expression so it can be overridden by RHOST_REMOTE_STATE on the host.
	DefaultRemoteStateDir = "$HOME/.local/state/rhost"
)

// NewNonce returns a 128-bit random hex token used to make the completion
// marker unforgeable by ordinary command output.
func NewNonce() (string, error) {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}

// ExecSpec describes one foreground remote execution.
type ExecSpec struct {
	Command string
	Cwd     string
	Env     map[string]string
	Nonce   string
	// RemoteStateDir is a shell expression; DefaultRemoteStateDir is used when
	// empty.
	RemoteStateDir string
}

func (s ExecSpec) stateExpr() string {
	if s.RemoteStateDir != "" {
		return s.RemoteStateDir
	}
	return DefaultRemoteStateDir
}

func markerToken(nonce string) string { return markerPrefix + nonce + "__" }

func beginToken(nonce string) string { return outBeginPrefix + nonce + "__" }

// BuildScript returns the bash script executed on the remote host.
//
// It is executed as a login shell (`bash -lc`) inside a new session
// (`setsid`), so `$$` is the session/process-group leader and the whole command
// can be killed with `kill -TERM -$$`. The script always exits 0; the command's
// real status is carried in the completion marker on stdout. That keeps the
// ssh-level exit status free to signal *transport* failure unambiguously.
func BuildScript(spec ExecSpec) string {
	return buildScript(spec, `\n`)
}

// BuildStreamScript uses a NUL boundary so a streaming parser never has to hold
// back a command's final newline while deciding whether a completion marker is
// about to follow. The nonce still makes the complete marker unforgeable by
// ordinary output.
func BuildStreamScript(spec ExecSpec) string {
	return buildScript(spec, `\000`)
}

func buildScript(spec ExecSpec, separator string) string {
	var b strings.Builder
	state := spec.stateExpr()

	b.WriteString(`RHOST_RD="` + state + "\"\n")
	b.WriteString("mkdir -p \"$RHOST_RD/run\" 2>/dev/null && RHOST_PD=\"$RHOST_RD/run\" || RHOST_PD=\"${TMPDIR:-/tmp}\"\n")
	b.WriteString("RHOST_PF=\"$RHOST_PD/rhost-" + spec.Nonce + ".pid\"\n")
	b.WriteString("printf '%s' \"$$\" > \"$RHOST_PF\" 2>/dev/null || RHOST_PF=\"\"\n")
	b.WriteString("trap 'rm -f \"$RHOST_PF\"' EXIT\n")

	if spec.Cwd != "" {
		q := shell.PathQuote(spec.Cwd)
		b.WriteString("cd -- " + q + " 2>/dev/null || { printf 'rhost: cannot change directory to %s\\n' " + q +
			" >&2; printf '\\n" + markerToken(spec.Nonce) + ":%d\\n' 126; exit 0; }\n")
	}

	keys := make([]string, 0, len(spec.Env))
	for k := range spec.Env {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		b.WriteString("export " + k + "=" + shell.Quote(spec.Env[k]) + "\n")
	}

	// Emit a begin marker immediately before the command's output. The wrapper
	// runs under `bash -lc`, which sources the login profile first; any stdout a
	// noisy profile produces would otherwise be prepended to the command's own
	// output. ParseMarker drops everything up to and including this marker.
	b.WriteString("printf '" + separator + beginToken(spec.Nonce) + "\\n'\n")

	// Run the user command in a subshell so a bare `exit` inside it cannot skip
	// the completion marker.
	b.WriteString("(\n")
	b.WriteString(spec.Command)
	if !strings.HasSuffix(spec.Command, "\n") {
		b.WriteString("\n")
	}
	b.WriteString(")\n")
	b.WriteString("RHOST_EC=$?\n")
	b.WriteString("printf '" + separator + markerToken(spec.Nonce) + ":%d\\n' \"$RHOST_EC\"\n")
	b.WriteString("exit 0\n")
	return b.String()
}

// WrapScript produces the command string handed to `ssh`. It runs the script
// through a login bash in a fresh session. The outer string is parsed by the
// remote account's login shell (which may be fish), so it uses only POSIX
// single-quote quoting.
func WrapScript(script string) string {
	// Only ASCII base64 crosses the account's login-shell parser. In particular,
	// fish and POSIX shells interpret backslashes inside single quotes differently.
	encoded := base64.StdEncoding.EncodeToString([]byte(script))
	return "exec setsid bash -lc " + shell.Quote("eval \"$(printf %s "+encoded+" | base64 -d)\"")
}

// ParseMarker extracts the completion marker emitted by BuildScript.
//
// It returns the command's stdout (with the separator newline, the begin marker,
// and the completion marker removed) and the remote exit code. ok is false when
// the marker is absent, which means the wrapper never completed (transport
// failure or a missing remote dependency such as bash/setsid).
func ParseMarker(stdout []byte, nonce string) (body []byte, code int, ok bool) {
	needle := []byte("\n" + markerPrefix + nonce + "__:")
	i := bytes.LastIndex(stdout, needle)
	if i < 0 {
		return stdout, -1, false
	}
	rest := stdout[i+len(needle):]
	j := bytes.IndexByte(rest, '\n')
	if j < 0 {
		return stdout, -1, false
	}
	n, err := strconv.Atoi(string(rest[:j]))
	if err != nil {
		return stdout, -1, false
	}
	head := stdout[:i]
	// Drop login-profile noise: everything up to and including the begin marker
	// is not the command's output. Match the first occurrence, which is the one
	// the wrapper emitted before the command ran.
	begin := []byte("\n" + beginToken(nonce) + "\n")
	if k := bytes.Index(head, begin); k >= 0 {
		head = head[k+len(begin):]
	}
	return head, n, true
}

// KillCommand returns a remote bash command that kills the process group
// recorded for nonce, escalating TERM -> KILL. Used to terminate a command whose
// foreground timeout elapsed; killing the local ssh process alone leaves the
// remote process running.
//
// BuildScript records the pid file under the remote state dir, but falls back to
// ${TMPDIR:-/tmp} when that dir is not writable. The killer must look in both
// places, or a command started on a host with an unwritable state dir can never
// be reaped.
func KillCommand(nonce string) string {
	name := "rhost-" + nonce + ".pid"
	return `f1="${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost}/run/` + name + `"; ` +
		`f2="${TMPDIR:-/tmp}/` + name + `"; ` +
		`p=$(cat "$f1" 2>/dev/null); ` +
		`[ -n "$p" ] || p=$(cat "$f2" 2>/dev/null); ` +
		`if [ -n "$p" ]; then ` +
		`kill -TERM -"$p" 2>/dev/null; sleep 0.3; kill -KILL -"$p" 2>/dev/null; ` +
		`rm -f "$f1" "$f2"; echo killed:$p; else echo no-pid; fi`
}
