package app

import (
	"context"
	"fmt"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/status"
)

// statusTimeout bounds one status refresh: the probe plus the session and job
// listings. It is longer than exec's default because the probe deliberately
// samples the CPU over ~0.3s and then reads three remote helpers.
const statusTimeout = 30 * time.Second

// StatusOptions describes one status/watch refresh.
type StatusOptions struct {
	Host    string
	Timeout time.Duration
}

// StatusResult is one snapshot of a remote host: generic system telemetry, the
// optional accelerators, and rhost's own managed sessions and jobs (§29).
//
// Online is always true for a successful `status`. `watch` reuses this type and
// sets Online=false (with OfflineCode/OfflineMessage) for a refresh that could
// not reach the host, so a single shape covers both the snapshot and the stream.
type StatusResult struct {
	Online         bool   `json:"online"`
	OfflineCode    string `json:"offline_code,omitempty"`
	OfflineMessage string `json:"offline_message,omitempty"`
	ProbedAt       string `json:"probed_at"`
	// ProbeMS is how long the snapshot's single remote probe took, wall-clock. It
	// is deliberately not called RTT: one bounded probe (§30) cannot separate
	// network round-trip time from remote execution time (the CPU sample alone
	// costs a fixed ~0.3s), and reporting that sum as an RTT would be a made-up
	// number. Reachability is Online.
	ProbeMS      int64                `json:"probe_ms"`
	System       status.System        `json:"system"`
	Accelerators []status.Accelerator `json:"accelerators"`
	Sessions     []SessionInfo        `json:"sessions"`
	Jobs         []JobInfo            `json:"jobs"`
	Unavailable  []string             `json:"unavailable"`
}

// Status returns one snapshot. An unreachable host is an adapter error here —
// the caller asked for a snapshot and none could be taken. `watch` wraps this
// and turns that error into an offline refresh instead.
func (a *App) Status(ctx context.Context, opts StatusOptions) (StatusResult, *errs.Error) {
	timeout := opts.Timeout
	if timeout <= 0 {
		timeout = statusTimeout
	}

	res, aerr := a.Execute(ctx, ExecOptions{
		Host:    opts.Host,
		Command: status.ProbeScript,
		Timeout: timeout,
	})
	if aerr != nil {
		return StatusResult{}, aerr
	}
	if res.ExitCode != 0 {
		return StatusResult{}, errs.New(errs.Internal,
			fmt.Sprintf("status probe exited %d", res.ExitCode), false)
	}
	snap, err := status.ParseProbe(res.Stdout)
	if err != nil {
		return StatusResult{}, errs.Wrap(errs.Internal,
			"unreadable status probe output: "+err.Error(), false, err)
	}

	// Sessions and jobs are rhost's own managed state; reuse the exact listing
	// code the session/job commands use rather than re-deriving it here.
	sessions, serr := a.SessionList(ctx, opts.Host, timeout)
	jobs, jerr := a.JobList(ctx, opts.Host, timeout)

	return combine(snap, res.Duration, time.Now(), sessions, serr, jobs, jerr), nil
}

// combine folds the optional session and job listings into the result. A listing
// failure degrades that section to empty and names it in Unavailable — a host
// with no tmux still has a system and jobs, and a snapshot that fails wholesale
// because one section is missing would be the opposite of §29's rule.
func combine(snap status.Snapshot, probe time.Duration, now time.Time, sessions []SessionInfo, serr *errs.Error, jobs []JobInfo, jerr *errs.Error) StatusResult {
	unavailable := append([]string{}, snap.Unavailable...)
	if serr != nil {
		unavailable = append(unavailable, "sessions: "+string(serr.Code))
	}
	if jerr != nil {
		unavailable = append(unavailable, "jobs: "+string(jerr.Code))
	}
	if sessions == nil {
		sessions = []SessionInfo{}
	}
	if jobs == nil {
		jobs = []JobInfo{}
	}
	if snap.Accelerators == nil {
		snap.Accelerators = []status.Accelerator{}
	}
	return StatusResult{
		Online:       true,
		ProbedAt:     now.UTC().Format(time.RFC3339),
		ProbeMS:      probe.Milliseconds(),
		System:       snap.System,
		Accelerators: snap.Accelerators,
		Sessions:     sessions,
		Jobs:         jobs,
		Unavailable:  unavailable,
	}
}
