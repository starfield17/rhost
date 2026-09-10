package app

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"regexp"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/session/tmux"
	"github.com/starfield17/rhost/internal/shell"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// SessionInfo is the CLI-facing view of a persistent session.
type SessionInfo struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	TmuxSession string `json:"tmux_session"`
	CreatedAt   string `json:"created_at"`
	InitialCwd  string `json:"initial_cwd,omitempty"`
	Shell       string `json:"shell"`
	Status      string `json:"status"` // alive | dead
}

// SessionExecResult is the outcome of running a command in a session.
type SessionExecResult struct {
	SessionID string
	Output    string
	ExitCode  int
	TimedOut  bool
}

// SessionReadResult is an incremental read of a session's output log.
type SessionReadResult struct {
	SessionID string
	From      int
	Next      int
	HasMore   bool
	Data      string
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

func sessionHelperTimeout(user time.Duration) time.Duration {
	if user <= 0 {
		user = 60 * time.Second
	}
	// Give the remote helper room to run its own timeout handling (Ctrl-C)
	// before the transport-level kill fires.
	return user + 20*time.Second
}

func mapHelperErr(code, host string) *errs.Error {
	switch code {
	case "nosession":
		return errs.New(errs.SessionNotFound, "no such session", false)
	case "sessiondied":
		return errs.New(errs.SessionNotFound, "session shell exited (the command likely ran `exit`)", false)
	case "timeout":
		return errs.New(errs.RemoteCommandTimeout, "command exceeded timeout in session", true)
	case "locked":
		return errs.New(errs.SessionUnhealthy, "session is busy (another writer holds the lock)", true)
	case "notready":
		return errs.New(errs.SessionUnhealthy, "session shell did not become ready", true)
	default:
		return errs.Wrap(errs.SessionUnhealthy, "session helper error: "+code, true, nil)
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
		return SessionInfo{}, errs.New(errs.ConfigInvalid, "only --shell bash is supported in v0.1", false)
	}
	id, err := newSessionID()
	if err != nil {
		return SessionInfo{}, errs.Wrap(errs.Internal, "could not generate session id", false, err)
	}
	if name == "" {
		name = id
	}
	if !sessionNameRe.MatchString(name) {
		return SessionInfo{}, errs.New(errs.ConfigInvalid, "invalid session name (use letters, digits, _ . -)", false)
	}

	meta := tmux.NewMeta(id, name, cwd, shellName)
	paneCmd := "bash --noprofile --norc -i"
	script := tmux.CreateScript(meta, paneCmd)

	if timeout <= 0 {
		timeout = 60 * time.Second
	}
	res, aerr := a.runHelper(ctx, host, script, timeout)
	if aerr != nil {
		return SessionInfo{}, aerr
	}
	stdout := string(res.Stdout)
	if strings.Contains(stdout, "RHOST_ERR=") {
		return SessionInfo{}, mapHelperErr(extractField(stdout, "RHOST_ERR="), host)
	}
	return toSessionInfo(meta, "alive"), nil
}

// SessionList lists known sessions and whether their tmux session is alive.
func (a *App) SessionList(ctx context.Context, host string, timeout time.Duration) ([]SessionInfo, *errs.Error) {
	res, aerr := a.runHelper(ctx, host, tmux.ListScript(), timeout)
	if aerr != nil {
		return nil, aerr
	}
	entries := tmux.ParseList(string(res.Stdout))
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
		return SessionExecResult{}, errs.New(errs.ConfigInvalid, "no command given", false)
	}
	if timeout <= 0 {
		timeout = 60 * time.Second
	}
	script := tmux.ExecScript(nameOrID, command, timeout)
	res, aerr := a.runHelper(ctx, host, script, sessionHelperTimeout(timeout))
	if aerr != nil {
		return SessionExecResult{}, aerr
	}
	oc := tmux.ParseExec(string(res.Stdout))
	if oc.Err != "" {
		return SessionExecResult{}, mapHelperErr(oc.Err, host)
	}
	return SessionExecResult{
		SessionID: nameOrID,
		Output:    shell.StripANSI(oc.Output),
		ExitCode:  oc.ExitCode,
	}, nil
}

// SessionSend injects raw data or a single key into a session.
func (a *App) SessionSend(ctx context.Context, host, nameOrID, data, key string, timeout time.Duration) *errs.Error {
	if (data == "") == (key == "") {
		return errs.New(errs.ConfigInvalid, "provide exactly one of --data or --key", false)
	}
	var script string
	if key != "" {
		if !validKeys[key] {
			return errs.New(errs.ConfigInvalid, "unsupported key "+key, false)
		}
		script = tmux.SendScript(nameOrID, "key", key)
	} else {
		script = tmux.SendScript(nameOrID, "data", data)
	}
	res, aerr := a.runHelper(ctx, host, script, timeout)
	if aerr != nil {
		return aerr
	}
	if strings.Contains(string(res.Stdout), "RHOST_ERR=") {
		return mapHelperErr(extractField(string(res.Stdout), "RHOST_ERR="), host)
	}
	return nil
}

// SessionRead reads the session output log incrementally.
func (a *App) SessionRead(ctx context.Context, host, nameOrID string, since int, timeout time.Duration) (SessionReadResult, *errs.Error) {
	res, aerr := a.runHelper(ctx, host, tmux.ReadScript(nameOrID, since, 0), timeout)
	if aerr != nil {
		return SessionReadResult{}, aerr
	}
	ro := tmux.ParseRead(string(res.Stdout))
	if ro.Error != "" {
		return SessionReadResult{}, mapHelperErr(ro.Error, host)
	}
	return SessionReadResult{
		SessionID: nameOrID,
		From:      ro.From,
		Next:      ro.Next,
		HasMore:   ro.Next < ro.Size,
		Data:      shell.StripANSI(string(ro.Data)),
	}, nil
}

// SessionClose kills a session and removes its remote state.
func (a *App) SessionClose(ctx context.Context, host, nameOrID string, timeout time.Duration) *errs.Error {
	res, aerr := a.runHelper(ctx, host, tmux.CloseScript(nameOrID), timeout)
	if aerr != nil {
		return aerr
	}
	if strings.Contains(string(res.Stdout), "RHOST_ERR=") {
		return mapHelperErr(extractField(string(res.Stdout), "RHOST_ERR="), host)
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
		return errs.Wrap(errs.SessionUnhealthy, "attach failed: "+err.Error(), true, err)
	}
	return nil
}

func toSessionInfo(m tmux.Meta, status string) SessionInfo {
	return SessionInfo{
		ID:          m.ID,
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
