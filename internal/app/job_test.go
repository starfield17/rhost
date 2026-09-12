package app

import (
	"context"
	"slices"
	"testing"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/job/detached"
)

// TestMapJobHelperErr pins the job helper codes to the taxonomy. Agents branch
// on error.code, so this mapping is a public contract (AGENTS.md §6).
func TestMapJobHelperErr(t *testing.T) {
	cases := []struct {
		herr      detached.HelperErr
		wantCode  errs.Code
		retryable bool
	}{
		{detached.NotFound, errs.JobNotFound, false},
		{"nobash", errs.RemoteDependencyMissing, false},
		{"nosetsid", errs.RemoteDependencyMissing, false},
		{"nonohup", errs.RemoteDependencyMissing, false},
		{"noproc", errs.RemoteDependencyMissing, false},
		{"mkdir", errs.JobStateUnknown, true},
		{"startfailed", errs.JobStateUnknown, true},
		{"malformed", errs.Internal, false},
		{"whatever-else", errs.JobStateUnknown, true},
	}
	for _, tc := range cases {
		got := mapJobHelperErr(tc.herr)
		if got.Code != tc.wantCode {
			t.Errorf("mapJobHelperErr(%q).Code = %s, want %s", tc.herr, got.Code, tc.wantCode)
		}
		if got.Retryable != tc.retryable {
			t.Errorf("mapJobHelperErr(%q).Retryable = %v, want %v", tc.herr, got.Retryable, tc.retryable)
		}
	}
}

// TestJobRefShape covers the validation that stands between a `<job>` argument
// and a generated remote script. The rejects are the interesting cases: `.` and
// `..` name real directories, and everything else here is shell syntax that must
// never reach a path.
func TestJobRefShape(t *testing.T) {
	accept := []string{"j_01ab23cd45ef", "train-1", "My_job.v2", "a", "_x", "j_nosuchjob"}
	reject := []string{
		"", ".", "..", "-flag",
		`x"; touch /tmp/pwned; #`,
		"x$(touch /tmp/pwned)",
		"x`touch /tmp/pwned`",
		"x;true",
		"a/b",
		"with space",
		"new\nline",
		"way-too-long-way-too-long-way-too-long-way-too-long-way-too-long-way-too-long",
	}
	for _, s := range accept {
		if !jobRefRe.MatchString(s) {
			t.Errorf("jobRefRe must accept %q", s)
		}
	}
	for _, s := range reject {
		if jobRefRe.MatchString(s) {
			t.Errorf("jobRefRe must reject %q", s)
		}
	}
}

func TestIsGeneratedJobID(t *testing.T) {
	yes := []string{"j_0123456789ab", "j_deadbeef0001", "j_000000000000"}
	no := []string{"j_nosuchjob", "train-1", "j_01Jabcdef012", "j_0123456789abc", "j_0123456789", "J_0123456789ab", "s_0123456789ab"}
	for _, s := range yes {
		if !isGeneratedJobID(s) {
			t.Errorf("isGeneratedJobID(%q) = false, want true", s)
		}
	}
	for _, s := range no {
		if isGeneratedJobID(s) {
			t.Errorf("isGeneratedJobID(%q) = true, want false", s)
		}
	}
}

// TestJobIDsByName is the whole name-resolution policy: a name is a convenience,
// and an ambiguous one is refused rather than guessed.
func TestJobIDsByName(t *testing.T) {
	facts := []detached.Facts{
		{ID: "j_1", Meta: &detached.Meta{Name: "train"}},
		{ID: "j_2", Meta: &detached.Meta{Name: "build"}},
		{ID: "j_3", Meta: &detached.Meta{Name: "train"}},
		{ID: "j_4"}, // a job dir whose metadata failed to read: never a match
	}

	if got := jobIDsByName(facts, "build"); !slices.Equal(got, []string{"j_2"}) {
		t.Errorf("unique name = %v, want [j_2]", got)
	}
	if got := jobIDsByName(facts, "train"); !slices.Equal(got, []string{"j_1", "j_3"}) {
		t.Errorf("ambiguous name = %v, want [j_1 j_3]", got)
	}
	if got := jobIDsByName(facts, "ghost"); got != nil {
		t.Errorf("unknown name = %v, want no matches", got)
	}
	// The same job appearing twice (a duplicated list row) is not a collision.
	dup := []detached.Facts{{ID: "j_9", Meta: &detached.Meta{Name: "x"}}, {ID: "j_9", Meta: &detached.Meta{Name: "x"}}}
	if got := jobIDsByName(dup, "x"); !slices.Equal(got, []string{"j_9"}) {
		t.Errorf("duplicate row = %v, want [j_9]", got)
	}
}

// A job name that is not a valid ref must be refused at start time, before
// anything touches the transport.
func TestJobStartRejectsBadNameWithoutTouchingHost(t *testing.T) {
	a := NewDefault()
	for _, name := range []string{".", "..", "-x", "a b", `a"b`, "j_0123456789ab"} {
		_, aerr := a.JobStart(context.Background(), JobStartOptions{Host: "unreachable-host-does-not-matter", Name: name, Command: "true"})
		if aerr == nil || aerr.Code != errs.ConfigInvalid {
			t.Errorf("JobStart(name=%q) = %+v, want CONFIG_INVALID", name, aerr)
		}
	}
}

// The same rule for the handle argument: an invalid shape is a local validation
// failure, never a remote command.
func TestJobRefIsValidatedBeforeAnySSH(t *testing.T) {
	a := NewDefault()
	const hostile = `x"; touch /tmp/pwned; #`
	ctx := context.Background()

	if _, aerr := a.JobStatus(ctx, "irrelevant-host", hostile, 0); aerr == nil || aerr.Code != errs.ConfigInvalid {
		t.Errorf("JobStatus = %+v, want CONFIG_INVALID", aerr)
	}
	if _, aerr := a.JobLogs(ctx, "irrelevant-host", hostile, "stdout", 0, 0); aerr == nil || aerr.Code != errs.ConfigInvalid {
		t.Errorf("JobLogs = %+v, want CONFIG_INVALID", aerr)
	}
	if _, aerr := a.JobSignal(ctx, "irrelevant-host", hostile, "TERM", 0); aerr == nil || aerr.Code != errs.ConfigInvalid {
		t.Errorf("JobSignal = %+v, want CONFIG_INVALID", aerr)
	}
	// A bad signal is refused for the same reason.
	if _, aerr := a.JobSignal(ctx, "irrelevant-host", "j_1", "STOP", 0); aerr == nil || aerr.Code != errs.ConfigInvalid {
		t.Errorf("JobSignal(STOP) = %+v, want CONFIG_INVALID", aerr)
	}
}
