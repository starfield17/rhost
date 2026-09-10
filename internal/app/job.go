package app

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"regexp"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/job/detached"
	"github.com/starfield17/rhost/internal/shell"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// JobStartOptions describes a detached background job.
type JobStartOptions struct {
	Host    string
	Name    string
	Cwd     string
	Command string
	Env     map[string]string
	Timeout time.Duration
}

// JobStartResult is the immediate outcome of launching a job.
type JobStartResult struct {
	ID    string `json:"id"`
	State string `json:"state"`
	PID   int    `json:"pid"`
}

// JobInfo is the CLI-facing view of one job.
type JobInfo struct {
	ID         string `json:"id"`
	Name       string `json:"name,omitempty"`
	State      string `json:"state"`
	PID        int    `json:"pid"`
	ExitCode   int    `json:"exit_code"` // -1 while not finished
	Command    string `json:"command"`
	Cwd        string `json:"cwd,omitempty"`
	StartedAt  string `json:"started_at"`
	FinishedAt string `json:"finished_at,omitempty"`
}

// JobLogsResult is an incremental read of a job's log stream.
type JobLogsResult struct {
	JobID    string `json:"job_id"`
	Stream   string `json:"stream"`
	From     int    `json:"from"`
	Next     int    `json:"next"`
	More     bool   `json:"more"`
	Encoding string `json:"encoding"` // "base64"
	Data     string `json:"data"`     // base64-encoded chunk
}

// JobSignalResult is the outcome of a stop/kill request.
type JobSignalResult struct {
	JobID  string `json:"job_id"`
	Signal string `json:"signal"`
	State  string `json:"state"`
}

func newJobID() (string, error) {
	b := make([]byte, 6)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return "j_" + hex.EncodeToString(b), nil
}

// mapJobHelperErr maps a remote helper failure code onto the taxonomy.
func mapJobHelperErr(code detached.HelperErr) *errs.Error {
	switch code {
	case detached.NotFound:
		return errs.New(errs.JobNotFound, "no such job", false)
	case "nobash", "nosetsid", "nonohup":
		dep := strings.TrimPrefix(string(code), "no")
		return errs.New(errs.RemoteDependencyMissing,
			"remote host is missing "+dep+" (required by the job backend)", false)
	case "mkdir":
		return errs.New(errs.JobStateUnknown, "could not create job state on the remote host", true)
	case "startfailed":
		return errs.New(errs.JobStateUnknown,
			"job process exited before registering; its state is unknown", true)
	case "malformed":
		return errs.New(errs.Internal, "job helper produced unreadable output", false)
	default:
		return errs.New(errs.JobStateUnknown, "job helper error: "+code.ErrString(), true)
	}
}

// runJobHelper executes a generated job script on the host.
func (a *App) runJobHelper(ctx context.Context, host, script string, timeout time.Duration) (openssh.Result, *errs.Error) {
	res, aerr := a.Execute(ctx, ExecOptions{Host: host, Command: script, Timeout: timeout})
	if aerr != nil {
		return openssh.Result{}, aerr
	}
	if res.ExitCode != 0 {
		return openssh.Result{}, errs.New(errs.JobStateUnknown,
			fmt.Sprintf("job helper exited %d", res.ExitCode), true)
	}
	return openssh.Result{Stdout: []byte(res.Stdout), Stderr: []byte(res.Stderr)}, nil
}

// jobRefRe is the shape a `<job>` argument may take: a generated id (`j_` plus
// hex) or a `--name`. Same character class as a session name, with one extra
// rule — the first character must be a letter or underscore, so `.` and `..`
// can never name a job directory. The app layer refuses anything else before a
// ref reaches a generated script; internal/job/detached quotes it as well.
var jobRefRe = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_.-]{0,63}$`)

// errInvalidJobRef is the taxonomy answer to a handle that is not a handle. It
// is validation of user input, not a usage/flag parse failure, so it is
// CONFIG_INVALID rather than USAGE_ERROR (docs/ARCHITECTURE.md §33).
func errInvalidJobRef(ref string) *errs.Error {
	return errs.New(errs.ConfigInvalid,
		fmt.Sprintf("invalid job id or name %q (start with a letter, then letters, digits, _ . -)", ref),
		false)
}

// resolveJobRef runs a per-job script against a handle, resolving it as a job id
// first and only as a `--name` when no job has that id. Names are a convenience
// for humans typing the command back in; the id remains the only unambiguous
// handle, and a name that matches more than one job is refused rather than
// guessed at.
//
// The id path costs one round trip: an unknown id is reported by the same script
// that would have read the job, so the retry only happens on that answer.
func (a *App) resolveJobRef(ctx context.Context, host, ref string, build func(string) string, timeout time.Duration) (string, openssh.Result, *errs.Error) {
	if !jobRefRe.MatchString(ref) {
		return "", openssh.Result{}, errInvalidJobRef(ref)
	}
	res, aerr := a.runJobHelper(ctx, host, build(ref), jobScriptTimeout(timeout))
	if aerr != nil {
		return "", res, aerr
	}
	if detached.ErrOf(string(res.Stdout)) != detached.NotFound {
		return ref, res, nil
	}

	id, aerr := a.jobIDByName(ctx, host, ref, timeout)
	if aerr != nil {
		return "", res, aerr
	}
	res, aerr = a.runJobHelper(ctx, host, build(id), jobScriptTimeout(timeout))
	if aerr != nil {
		return "", res, aerr
	}
	return id, res, nil
}

// jobIDByName maps a job name to its id, failing when the name is unknown or
// ambiguous.
func (a *App) jobIDByName(ctx context.Context, host, name string, timeout time.Duration) (string, *errs.Error) {
	res, aerr := a.runJobHelper(ctx, host, detached.ListScript(), jobScriptTimeout(timeout))
	if aerr != nil {
		return "", aerr
	}
	// The decision is a switch on the number of matches, so "use the first one"
	// is not expressible here: an ambiguous name must never quietly signal the
	// wrong job.
	switch ids := jobIDsByName(detached.ParseList(string(res.Stdout)), name); len(ids) {
	case 0:
		return "", errs.New(errs.JobNotFound, "no job with this id or name", false)
	case 1:
		return ids[0], nil
	default:
		return "", errs.New(errs.ConfigInvalid,
			fmt.Sprintf("job name %q matches %d jobs (%s): use the job id", name, len(ids), strings.Join(ids, ", ")),
			false)
	}
}

// jobIDsByName returns every job id carrying this name. Returning the full set
// rather than "the match" is what keeps the ambiguity decision honest, and it is
// a pure function so the rule is testable without a host.
//
// A job dir whose metadata could not be read has no name to match, and two rows
// for the same job are not a collision.
func jobIDsByName(facts []detached.Facts, name string) []string {
	var ids []string
	for _, f := range facts {
		if f.Meta == nil || f.Meta.Name != name || slices.Contains(ids, f.ID) {
			continue
		}
		ids = append(ids, f.ID)
	}
	return ids
}

// JobStart launches a detached background job and returns its immediate state
// (never a fake "running": the state comes from the same facts every later
// status call observes).
func (a *App) JobStart(ctx context.Context, opts JobStartOptions) (JobStartResult, *errs.Error) {
	if strings.TrimSpace(opts.Command) == "" {
		return JobStartResult{}, errs.New(errs.ConfigInvalid, "no command given", false)
	}
	for k := range opts.Env {
		if err := shell.ValidateEnvKey(k); err != nil {
			return JobStartResult{}, errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
		}
	}
	id, err := newJobID()
	if err != nil {
		return JobStartResult{}, errs.Wrap(errs.Internal, "could not generate job id", false, err)
	}
	name := opts.Name
	if name == "" {
		name = id
	}
	if !jobRefRe.MatchString(name) {
		return JobStartResult{}, errs.New(errs.ConfigInvalid, "invalid job name (start with a letter, then letters, digits, _ . -)", false)
	}

	meta := detached.NewMeta(id, name, opts.Cwd, opts.Command)
	cmdScript := detached.CommandScript(id, opts.Cwd, opts.Env, opts.Command)

	if opts.Timeout <= 0 {
		opts.Timeout = 60 * time.Second
	}
	res, aerr := a.runJobHelper(ctx, opts.Host, detached.StartScript(meta, cmdScript), opts.Timeout)
	if aerr != nil {
		return JobStartResult{}, aerr
	}
	facts, herr := detached.ParseStart(string(res.Stdout))
	if herr != "" {
		return JobStartResult{}, mapJobHelperErr(herr)
	}
	return JobStartResult{
		ID:    facts.ID,
		State: string(detached.StateFromFacts(facts)),
		PID:   facts.PID,
	}, nil
}

// JobStatus reports one job's derived state. `ref` is a job id or a unique name.
func (a *App) JobStatus(ctx context.Context, host, ref string, timeout time.Duration) (JobInfo, *errs.Error) {
	id, res, aerr := a.resolveJobRef(ctx, host, ref, detached.StatusScript, timeout)
	if aerr != nil {
		return JobInfo{}, aerr
	}
	facts, herr := detached.ParseStatus(string(res.Stdout))
	if herr != "" {
		return JobInfo{}, mapJobHelperErr(herr)
	}
	info := toJobInfo(facts)
	info.ID = id
	return info, nil
}

// JobList lists all known jobs and their derived states.
func (a *App) JobList(ctx context.Context, host string, timeout time.Duration) ([]JobInfo, *errs.Error) {
	res, aerr := a.runJobHelper(ctx, host, detached.ListScript(), jobScriptTimeout(timeout))
	if aerr != nil {
		return nil, aerr
	}
	factsList := detached.ParseList(string(res.Stdout))
	sort.Slice(factsList, func(i, j int) bool { return factsList[i].ID < factsList[j].ID })
	out := make([]JobInfo, 0, len(factsList))
	for _, f := range factsList {
		out = append(out, toJobInfo(f))
	}
	return out, nil
}

// JobLogs reads a job log stream incrementally. Data is base64-encoded in the
// result so binary log content survives JSON untouched (docs/ARCHITECTURE.md
// §24). `ref` is a job id or a unique name.
func (a *App) JobLogs(ctx context.Context, host, ref, stream string, since int, timeout time.Duration) (JobLogsResult, *errs.Error) {
	if stream != "stdout" && stream != "stderr" {
		return JobLogsResult{}, errs.New(errs.ConfigInvalid, "stream must be stdout or stderr", false)
	}
	id, res, aerr := a.resolveJobRef(ctx, host, ref, func(s string) string {
		return detached.LogsScript(s, stream, since, 0)
	}, timeout)
	if aerr != nil {
		return JobLogsResult{}, aerr
	}
	lo, herr := detached.ParseLogs(string(res.Stdout))
	if herr != "" {
		return JobLogsResult{}, mapJobHelperErr(herr)
	}
	return JobLogsResult{
		JobID:    id,
		Stream:   stream,
		From:     lo.From,
		Next:     lo.Next,
		More:     lo.Next < lo.Size,
		Encoding: "base64",
		Data:     base64.StdEncoding.EncodeToString(lo.Data),
	}, nil
}

// JobSignal stops (TERM) or kills (KILL) a job's process group. Both are
// idempotent: an already-final job satisfies the requested condition. `ref` is a
// job id or a unique name.
func (a *App) JobSignal(ctx context.Context, host, ref, signal string, timeout time.Duration) (JobSignalResult, *errs.Error) {
	if signal != "TERM" && signal != "KILL" {
		return JobSignalResult{}, errs.New(errs.ConfigInvalid, "signal must be TERM or KILL", false)
	}
	id, res, aerr := a.resolveJobRef(ctx, host, ref, func(s string) string {
		return detached.SignalScript(s, signal == "KILL")
	}, timeout)
	if aerr != nil {
		return JobSignalResult{}, aerr
	}
	facts, herr := detached.ParseStatus(string(res.Stdout))
	if herr != "" {
		return JobSignalResult{}, mapJobHelperErr(herr)
	}
	return JobSignalResult{
		JobID:  id,
		Signal: signal,
		State:  string(detached.StateFromFacts(facts)),
	}, nil
}

// jobScriptTimeout is the transport budget for the status/log/signal helpers.
// Each is a short read of local files plus at most a signal; no user command
// runs, so a few seconds is generous.
func jobScriptTimeout(user time.Duration) time.Duration {
	if user <= 0 {
		return 30 * time.Second
	}
	return user
}

func toJobInfo(f detached.Facts) JobInfo {
	info := JobInfo{
		ID:         f.ID,
		State:      string(detached.StateFromFacts(f)),
		PID:        f.PID,
		ExitCode:   f.ExitCode,
		StartedAt:  "",
		FinishedAt: f.FinishedAt,
	}
	if f.Meta != nil {
		info.Name = f.Meta.Name
		info.Command = f.Meta.Command
		info.Cwd = f.Meta.Cwd
		info.StartedAt = f.Meta.StartedAt
	}
	return info
}
