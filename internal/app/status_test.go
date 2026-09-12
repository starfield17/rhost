package app

import (
	"strings"
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/status"
)

// combine is the part of Status that decides what one refresh contains. It is
// pure, so the degradation rule — one unreadable section must not fail the whole
// snapshot (§29) — is pinned without a host.

func TestCombineHealthy(t *testing.T) {
	snap := status.Snapshot{Unavailable: []string{"cpu_percent"}}
	got := combine(snap, 12*time.Millisecond, time.Unix(0, 0),
		[]SessionInfo{{ID: "s_1"}}, nil, []JobInfo{{ID: "j_1"}}, nil)

	if !got.Online {
		t.Error("a successful refresh must be online")
	}
	if got.ProbeMS != 12 {
		t.Errorf("probe_ms = %d, want 12", got.ProbeMS)
	}
	if got.ProbedAt == "" {
		t.Error("probed_at must be set")
	}
	if len(got.Sessions) != 1 || len(got.Jobs) != 1 {
		t.Errorf("sections not carried through: %+v", got)
	}
	if len(got.Unavailable) != 1 || got.Unavailable[0] != "cpu_percent" {
		t.Errorf("unavailable = %v, want the snapshot's own list", got.Unavailable)
	}
}

func TestCombineDegradesPerSection(t *testing.T) {
	got := combine(status.Snapshot{}, 0, time.Unix(0, 0),
		nil, errs.New(errs.RemoteDependencyMissing, "no tmux", false),
		nil, errs.New(errs.JobStateUnknown, "no jobs dir", true))

	// The snapshot still succeeds; the missing sections are named, not fatal.
	if !got.Online {
		t.Error("a section failure must not take the snapshot offline")
	}
	if len(got.Sessions) != 0 || len(got.Jobs) != 0 {
		t.Errorf("failed sections must be empty, got %+v", got)
	}
	want := map[string]bool{"sessions: REMOTE_DEPENDENCY_MISSING": true, "jobs: JOB_STATE_UNKNOWN": true}
	for _, u := range got.Unavailable {
		delete(want, u)
	}
	if len(want) != 0 {
		t.Errorf("unavailable = %v, missing %v", got.Unavailable, want)
	}
}

// A nil sessions/jobs slice must serialise as [] rather than null, so an agent
// can iterate without a nil check.
func TestCombineNeverEmitsNilSlices(t *testing.T) {
	got := combine(status.Snapshot{}, 0, time.Unix(0, 0), nil, nil, nil, nil)
	if got.Sessions == nil || got.Jobs == nil || got.Accelerators == nil {
		t.Errorf("nil slices must be normalised to empty: %+v", got)
	}
}

func TestSplitSnapshot(t *testing.T) {
	out := "rhost_probe_version=1\nhostname=box\nos=Linux\n" +
		snapshotSessions + "\nRHOST_ERR=notmux\n" +
		snapshotJobs + "\nRHOST_META\tj_1\t9\tyes\tverified\t-1\tno\t\t\n"
	probe, sessions, jobs := splitSnapshot(out)
	if !strings.Contains(probe, "hostname=box") || strings.Contains(probe, "RHOST_ERR") {
		t.Errorf("probe section = %q", probe)
	}
	sessionsOut, serr := sessionsFromList(sessions)
	if serr == nil || serr.Code != errs.RemoteDependencyMissing {
		t.Errorf("sessions error = %+v, want REMOTE_DEPENDENCY_MISSING", serr)
	}
	if len(sessionsOut) != 0 {
		t.Errorf("failed sessions section must be empty, got %+v", sessionsOut)
	}
	gotJobs := jobsFromList(jobs)
	if len(gotJobs) != 1 || gotJobs[0].ID != "j_1" {
		t.Errorf("jobs = %+v, want one row for j_1", gotJobs)
	}
}

func TestSplitSnapshotMissingMarkers(t *testing.T) {
	probe, sessions, jobs := splitSnapshot("rhost_probe_version=1\nhostname=box\n")
	if !strings.Contains(probe, "hostname=box") {
		t.Errorf("probe = %q", probe)
	}
	if sessions != "" || jobs != "" {
		t.Errorf("missing markers should leave sections empty: %q %q", sessions, jobs)
	}
}

func TestSnapshotScriptContainsSections(t *testing.T) {
	s := snapshotScript()
	if !strings.Contains(s, snapshotSessions) || !strings.Contains(s, snapshotJobs) {
		t.Fatal("snapshot script missing section markers")
	}
	if !strings.Contains(s, "rhost_probe_version=") {
		t.Fatal("snapshot script missing the system probe")
	}
	sess := strings.Index(s, snapshotSessions)
	jobs := strings.Index(s, snapshotJobs)
	if sess < 0 || jobs <= sess {
		t.Fatalf("section order: sessions at %d, jobs at %d", sess, jobs)
	}
	if !strings.Contains(s[sess:jobs], "(") {
		t.Fatal("session list must run in a subshell so its exit cannot skip jobs")
	}
}
