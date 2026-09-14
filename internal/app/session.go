package app

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"regexp"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/config"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/session/tmux"
	"github.com/starfield17/rhost/internal/shell"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// SessionInfo is the CLI-facing view of a persistent session.
type SessionInfo struct {
	SessionID   string `json:"session_id"`
	Name        string `json:"name"`
	TmuxSession string `json:"tmux_session"`
	CreatedAt   string `json:"created_at"`
	InitialCwd  string `json:"initial_cwd,omitempty"`
	Shell       string `json:"shell"`
	Status      string `json:"status"` // alive | dead | unknown
}

// SessionExecResult is the outcome of running a command in a session.
type SessionExecResult struct {
	SessionID  string
	SessionRef string
	Output     string
	// ExitCode is nil unless this invocation produced token-bound completion
	// evidence. A protocol failure must never acquire a default successful code.
	ExitCode *int
	TimedOut bool
	// SessionPreserved is the answer to the only question that matters after a
	// timeout: is this session still usable, or is something still running in it?
	// True means the pane was observed returning to a prompt.
	SessionPreserved bool
}

// SessionRecoverResult reports whether recover observed the managed shell at a
// fresh prompt. Foreground is present when another program still owns the pane.
type SessionRecoverResult struct {
	SessionID        string
	SessionRef       string
	Foreground       string
	SessionPreserved bool
}

// SessionReadResult is an incremental read of a session's output log.
type SessionReadResult struct {
	SessionID  string
	SessionRef string
	From       int
	Next       int
	HasMore    bool
	Data       string
}

var sessionNameRe = regexp.MustCompile(`^[A-Za-z0-9_.-]{1,64}$`)

// validKey reports whether a tmux key name is on the small allowlist rhost
// accepts. This prevents arbitrary text from being injected as a key name.
var validKeys = map[string]bool{
	"C-c": true, "C-d": true, "C-z": true, "C-\\": true, "C-u": true,
	"C-l": true, "C-a": true, "C-e": true, "C-w": true,
	"Enter": true, "Tab": true, "BTab": true, "Space": true,
	"BSpace": true, "Escape": true,
	"Up": true, "Down": true, "Left": true, "Right": true,
	"Home": true, "End": true, "PageUp": true, "PageDown": true,
}

func newSessionID() (string, error) {
	b := make([]byte, 6)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return "s_" + hex.EncodeToString(b), nil
}

func newSessionToken() (string, error) {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return hex.EncodeToString(b), nil
}

// sessionHelperTimeout is the transport-level budget for one session helper.
//
// The helper's own worst case is bounded by: a non-blocking flock, up to 5s
// waiting for the pane to go idle, then the user command's timeout. If the
// transport kills the helper before it can finish its own timeout handling
// (send C-c, release the lock), the pane and lock are left in an inconsistent
// state, so give the helper room for those phases.
func sessionHelperTimeout(user time.Duration) time.Duration {
	if user <= 0 {
		user = 60 * time.Second
	}
	const (
		lockWait = 2 * time.Second
		idleWait = 5 * time.Second
		slack    = 10 * time.Second
	)
	return user + lockWait + idleWait + slack
}

func mapHelperErr(code string) *errs.Error {
	switch code {
	case "nosession":
		return errs.New(errs.SessionNotFound, "no such session", false)
	case "sessiondied":
		return errs.New(errs.SessionNotFound, "session shell exited (the command likely ran `exit`)", false)
	case "timeout":
		return errs.New(errs.RemoteCommandTimeout, "command exceeded timeout in session", false)
	case "locked":
		return errs.New(errs.SessionUnhealthy, "session is busy (another writer holds the lock)", true)
	case "inputfailed":
		return errs.New(errs.SessionUnhealthy, "could not submit command input to session", true)
	case "busy":
		return errs.New(errs.SessionBusy,
			"session foreground is not the managed shell; use session send/read, or session recover", true)
	case "unknownfg":
		return errs.New(errs.SessionUnhealthy, "could not read the pane's foreground command", true)
	case "notty":
		return errs.New(errs.SessionUnhealthy, "could not open the pane's terminal", true)
	case "notready":
		return errs.New(errs.SessionUnhealthy, "session shell did not become ready", true)
	case "notmux":
		return errs.New(errs.RemoteDependencyMissing, "remote host is missing tmux", false)
	case "noflock":
		return errs.New(errs.RemoteDependencyMissing, "remote host is missing flock (util-linux)", false)
	case "nameinuse":
		return errs.New(errs.ConfigInvalid, "session name is already in use", false)
	case "invalidcwd":
		return errs.New(errs.ConfigInvalid, "initial working directory is not accessible", false)
	case "newfailed":
		return errs.New(errs.SessionUnhealthy, "could not create the tmux session", true)
	case "protocol":
		return errs.New(errs.SessionUnhealthy, "session helper returned incomplete or invalid completion evidence", true)
	default:
		return errs.New(errs.SessionUnhealthy, "session helper error: "+code, true)
	}
}

// runHelper executes a generated session script on the host.
func (a *App) runHelper(ctx context.Context, host, script string, timeout time.Duration) (openssh.Result, *errs.Error) {
	res, aerr := a.Execute(ctx, ExecOptions{Host: host, Command: script, Timeout: timeout})
	if aerr != nil {
		return openssh.Result{}, aerr
	}
	if res.ExitCode != 0 {
		return openssh.Result{}, errs.New(errs.SessionUnhealthy,
			fmt.Sprintf("session helper exited %d", res.ExitCode), true)
	}
	return openssh.Result{Stdout: []byte(res.Stdout), Stderr: []byte(res.Stderr)}, nil
}

// SessionCreate creates a tmux-backed persistent session.
func (a *App) SessionCreate(ctx context.Context, host, name, cwd, shellName string, timeout time.Duration) (SessionInfo, *errs.Error) {
	if shellName == "" {
		shellName = tmux.DefaultShell
	}
	if shellName != "bash" {
		return SessionInfo{}, errs.New(errs.ConfigInvalid, "only --shell bash is supported", false)
	}
	if name != "" && !sessionNameRe.MatchString(name) {
		return SessionInfo{}, errs.New(errs.ConfigInvalid, "invalid session name (use letters, digits, _ . -)", false)
	}
	id, err := newSessionID()
	if err != nil {
		return SessionInfo{}, errs.Wrap(errs.Internal, "could not generate session id", false, err)
	}
	if name == "" {
		name = id
	}

	meta := tmux.NewMeta(id, name, cwd, shellName)
	paneCmd := "bash --noprofile --norc -i"
	script := tmux.CreateScript(meta, paneCmd)

	if timeout <= 0 {
		timeout = 60 * time.Second
	}
	res, aerr := a.runHelper(ctx, host, script, timeout)
	if aerr != nil {
		return toSessionInfo(meta, "unknown"), aerr
	}
	stdout := string(res.Stdout)
	if strings.Contains(stdout, "RHOST_ERR=") {
		return SessionInfo{}, mapHelperErr(extractField(stdout, "RHOST_ERR="))
	}
	return toSessionInfo(meta, "alive"), nil
}

// SessionList lists known sessions and whether their tmux session is alive.
func (a *App) SessionList(ctx context.Context, host string, timeout time.Duration) ([]SessionInfo, *errs.Error) {
	res, aerr := a.runHelper(ctx, host, tmux.ListScript(), timeout)
	if aerr != nil {
		return nil, aerr
	}
	return sessionsFromList(string(res.Stdout))
}

// sessionsFromList is the shared parser for `session list` and the sessions
// session list response.
func sessionsFromList(stdout string) ([]SessionInfo, *errs.Error) {
	if strings.Contains(stdout, "RHOST_ERR=") {
		return nil, mapHelperErr(extractField(stdout, "RHOST_ERR="))
	}
	entries := tmux.ParseList(stdout)
	out := make([]SessionInfo, 0, len(entries))
	for _, e := range entries {
		status := "dead"
		if e.Alive {
			status = "alive"
		}
		out = append(out, toSessionInfo(e.Meta, status))
	}
	return out, nil
}

// SessionExec runs a command in a session and returns its output and status.
func (a *App) SessionExec(ctx context.Context, host, nameOrID, command string, timeout time.Duration) (SessionExecResult, *errs.Error) {
	if strings.TrimSpace(command) == "" {
		return SessionExecResult{SessionRef: nameOrID}, errs.New(errs.ConfigInvalid, "no command given", false)
	}
	if timeout <= 0 {
		timeout = 60 * time.Second
	}
	token, err := newSessionToken()
	if err != nil {
		return SessionExecResult{SessionRef: nameOrID},
			errs.Wrap(errs.Internal, "could not generate session command token", false, err)
	}
	script := tmux.ExecScript(nameOrID, command, timeout, token)
	res, aerr := a.runHelper(ctx, host, script, sessionHelperTimeout(timeout))
	if aerr != nil {
		return SessionExecResult{SessionRef: nameOrID}, aerr
	}
	oc := tmux.ParseExec(string(res.Stdout), token)
	if oc.Err != "" {
		// The session survived the helper's own interrupt only if the pane came back
		// to a prompt, so `session_preserved` is the helper's answer, not a guess
		// made from the exit status of the CLI that gave up.
		return SessionExecResult{
			SessionID:        oc.SessionID,
			SessionRef:       nameOrID,
			TimedOut:         oc.Err == "timeout",
			SessionPreserved: oc.Recovered,
		}, mapSessionExecErr(oc)
	}
	code := oc.ExitCode
	return SessionExecResult{
		SessionID:        oc.SessionID,
		SessionRef:       nameOrID,
		SessionPreserved: true,
		Output:           shell.StripANSI(oc.Output),
		ExitCode:         &code,
	}, nil
}

// SessionRecover interrupts a session under the same remote writer lock as
// exec/send. It never types an exit command into a surviving REPL.
func (a *App) SessionRecover(ctx context.Context, host, nameOrID string, timeout time.Duration) (SessionRecoverResult, *errs.Error) {
	if timeout <= 0 {
		timeout = 30 * time.Second
	}
	result := SessionRecoverResult{SessionRef: nameOrID}
	res, aerr := a.runHelper(ctx, host, tmux.RecoverScript(nameOrID, timeout), sessionHelperTimeout(timeout))
	if aerr != nil {
		return result, aerr
	}
	stdout := string(res.Stdout)
	result.SessionID = extractField(stdout, "RHOST_ID=")
	if code := extractField(stdout, "RHOST_ERR="); code != "" {
		result.Foreground = extractField(stdout, "RHOST_FG=")
		if code == "busy" {
			return result, mapSessionExecErr(tmux.ExecOutcome{Err: code, Foreground: result.Foreground})
		}
		return result, mapHelperErr(code)
	}
	if result.SessionID == "" || !strings.Contains(stdout, "RHOST_OK=recovered") {
		return result, mapHelperErr("protocol")
	}
	result.SessionPreserved = true
	return result, nil
}

// mapSessionExecErr turns an exec helper failure into the taxonomy. `busy` is the
// one case that carries detail worth keeping: the caller has to know *what* owns
// the pane, because the next step is to drive that program (session send/read) or
// to interrupt it (session recover) — never to retry the same paste.
func mapSessionExecErr(oc tmux.ExecOutcome) *errs.Error {
	if oc.Err != "busy" {
		return mapHelperErr(oc.Err)
	}
	what := oc.Foreground
	if what == "" {
		what = "another program"
	}
	return errs.New(errs.SessionBusy,
		"session foreground is "+what+", not the managed shell; use session send/read, or session recover", true)
}

// SessionSend injects raw data or a single key into a session.
func (a *App) SessionSend(ctx context.Context, host, nameOrID, data, key string, enter bool, timeout time.Duration) *errs.Error {
	if (data == "") == (key == "") || (key != "" && enter) {
		return errs.New(errs.ConfigInvalid, "provide --data [--enter] or exactly one --key", false)
	}
	var script string
	if key != "" {
		if !validKeys[key] {
			return errs.New(errs.ConfigInvalid, "unsupported key "+key, false)
		}
		script = tmux.SendScript(nameOrID, "key", key)
	} else {
		kind := "data"
		if enter {
			kind = "data-enter"
		}
		script = tmux.SendScript(nameOrID, kind, data)
	}
	res, aerr := a.runHelper(ctx, host, script, timeout)
	if aerr != nil {
		return aerr
	}
	if strings.Contains(string(res.Stdout), "RHOST_ERR=") {
		return mapHelperErr(extractField(string(res.Stdout), "RHOST_ERR="))
	}
	return nil
}

// SessionRead reads the session output log incrementally.
func (a *App) SessionRead(ctx context.Context, host, nameOrID string, since int, timeout time.Duration) (SessionReadResult, *errs.Error) {
	res, aerr := a.runHelper(ctx, host, tmux.ReadScript(nameOrID, since, 0), timeout)
	if aerr != nil {
		return SessionReadResult{SessionRef: nameOrID}, aerr
	}
	ro := tmux.ParseRead(string(res.Stdout))
	if ro.Error != "" {
		return SessionReadResult{SessionID: ro.SessionID, SessionRef: nameOrID}, mapHelperErr(ro.Error)
	}
	// A hard cut at the read byte limit can land mid-rune. Hold the partial
	// trailing bytes back and let the next read re-deliver them, so JSON never
	// carries a replacement character produced by rhost's own boundary.
	data, next := trimPartialRead(ro.Data, ro.Next, ro.Size)
	return SessionReadResult{
		SessionID:  ro.SessionID,
		SessionRef: nameOrID,
		From:       ro.From,
		Next:       next,
		HasMore:    next < ro.Size,
		Data:       shell.StripANSI(string(data)),
	}, nil
}

// trimPartialRead holds back a trailing partial rune when a read stopped at the
// byte limit (next < size) rather than at EOF, and moves the cursor back by the
// same amount so the bytes are re-delivered once complete. A read that reached
// EOF is returned unchanged: trailing bytes there are whatever the log holds.
func trimPartialRead(data []byte, next, size int) ([]byte, int) {
	if next >= size {
		return data, next
	}
	if n := shell.IncompleteUTF8Suffix(data); n > 0 {
		return data[:len(data)-n], next - n
	}
	return data, next
}

// SessionClose kills a session and removes its remote state.
func (a *App) SessionClose(ctx context.Context, host, nameOrID string, timeout time.Duration) *errs.Error {
	res, aerr := a.runHelper(ctx, host, tmux.CloseScript(nameOrID), timeout)
	if aerr != nil {
		return aerr
	}
	if strings.Contains(string(res.Stdout), "RHOST_ERR=") {
		return mapHelperErr(extractField(string(res.Stdout), "RHOST_ERR="))
	}
	return nil
}

// SessionAttach attaches the local terminal to a session's tmux pane. Echo is
// enabled for the duration of the attach (rhost keeps it disabled for clean
// agent capture) and disabled again afterwards.
func (a *App) SessionAttach(ctx context.Context, host, nameOrID string) *errs.Error {
	// Resolve the tmux session name on the remote side first.
	res, aerr := a.runHelper(ctx, host, tmux.ListScript(), 60*time.Second)
	if aerr != nil {
		return aerr
	}
	var tmuxName string
	for _, e := range tmux.ParseList(string(res.Stdout)) {
		if e.ID == nameOrID || e.Meta.Name == nameOrID {
			tmuxName = e.Meta.TmuxSession
			break
		}
	}
	if tmuxName == "" {
		return errs.New(errs.SessionNotFound, "no such session", false)
	}

	// Best-effort echo on; ignore failures so attach still works.
	_, _ = a.runHelper(ctx, host, tmux.EchoScript(nameOrID, true), 30*time.Second)
	defer func() {
		// Use a fresh context: the attach context may be cancelled on detach.
		bg, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_, _ = a.runHelper(bg, host, tmux.EchoScript(nameOrID, false), 30*time.Second)
	}()

	if err := a.SSH.RunInteractive(ctx, host, "tmux attach-session -t "+shell.Quote(tmuxName)); err != nil {
		if errors.Is(err, config.ErrUnsafeLocalState) {
			return errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
		}
		return errs.Wrap(errs.SessionUnhealthy, "attach failed: "+err.Error(), true, err)
	}
	return nil
}

func toSessionInfo(m tmux.Meta, status string) SessionInfo {
	return SessionInfo{
		SessionID:   m.ID,
		Name:        m.Name,
		TmuxSession: m.TmuxSession,
		CreatedAt:   m.CreatedAt,
		InitialCwd:  m.InitialCwd,
		Shell:       m.Shell,
		Status:      status,
	}
}

func extractField(stdout, key string) string {
	for _, line := range strings.Split(stdout, "\n") {
		if strings.HasPrefix(line, key) {
			return strings.TrimSpace(strings.TrimPrefix(line, key))
		}
	}
	return ""
}
