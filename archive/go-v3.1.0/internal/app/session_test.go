package app

import (
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/errs"
)

// TestMapHelperErr pins the remote helper error strings to taxonomy codes.
// Agents branch on Code, so the mapping is a contract.
func TestMapHelperErr(t *testing.T) {
	cases := []struct {
		code     string
		wantCode errs.Code
	}{
		{"nosession", errs.SessionNotFound},
		{"sessiondied", errs.SessionNotFound},
		{"timeout", errs.RemoteCommandTimeout},
		{"locked", errs.SessionUnhealthy},
		{"notready", errs.SessionUnhealthy},
		{"notmux", errs.RemoteDependencyMissing},
		{"noflock", errs.RemoteDependencyMissing},
		{"nameinuse", errs.ConfigInvalid},
		{"newfailed", errs.SessionUnhealthy},
		{"something-else", errs.SessionUnhealthy},
	}
	for _, tc := range cases {
		if got := mapHelperErr(tc.code); got.Code != tc.wantCode {
			t.Errorf("mapHelperErr(%q).Code = %s, want %s", tc.code, got.Code, tc.wantCode)
		}
	}
}

func TestMapHelperErrNamesInputSubmissionFailure(t *testing.T) {
	got := mapHelperErr("inputfailed")
	if got.Code != errs.SessionUnhealthy || got.Message != "could not submit command input to session" {
		t.Errorf("mapHelperErr(inputfailed) = %+v", got)
	}
}

// TestSessionHelperTimeoutCoversHelperPhases guards the timeout budget: the
// transport must not kill the helper while it is still waiting on the lock, the
// idle pane, or the user command's own timeout.
func TestSessionHelperTimeoutCoversHelperPhases(t *testing.T) {
	user := 30 * time.Second
	const phases = 2*time.Second + 5*time.Second // non-blocking lock + idle wait
	if got, want := sessionHelperTimeout(user), user+phases+10*time.Second; got != want {
		t.Errorf("sessionHelperTimeout(%s) = %s, want %s", user, got, want)
	}
	if got := sessionHelperTimeout(0); got <= 60*time.Second {
		t.Errorf("sessionHelperTimeout(0) = %s, want the default command budget plus helper phases", got)
	}
}

// TestTrimPartialRead pins the session-read UTF-8 boundary fix: a read cut at
// the byte limit must not emit a partial rune, and the cursor must move back so
// the bytes are re-delivered; a read at EOF is left alone.
func TestTrimPartialRead(t *testing.T) {
	// 3 ASCII bytes then the first 2 bytes of a 3-byte rune, cut at the limit.
	data := []byte{'a', 'b', 'c', 0xE2, 0x82}

	got, next := trimPartialRead(data, 5, 100)
	if string(got) != "abc" {
		t.Errorf("data = %q, want %q", got, "abc")
	}
	if next != 3 {
		t.Errorf("next = %d, want 3", next)
	}

	// At EOF (next == size) the trailing bytes are returned unchanged.
	got, next = trimPartialRead(data, 5, 5)
	if string(got) != string(data) || next != 5 {
		t.Errorf("at EOF: data=%q next=%d, want unchanged", got, next)
	}

	// A complete trailing rune is never held back.
	complete := []byte{'a', 0xE2, 0x82, 0xAC}
	got, next = trimPartialRead(complete, 4, 100)
	if string(got) != string(complete) || next != 4 {
		t.Errorf("complete rune: data=%q next=%d, want unchanged", got, next)
	}
}
