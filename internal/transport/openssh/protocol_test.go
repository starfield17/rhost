package openssh

import (
	"strings"
	"testing"
)

func TestBuildScriptContainsProtocol(t *testing.T) {
	s := BuildScript(ExecSpec{
		Command: "echo hi",
		Cwd:     "/tmp",
		Env:     map[string]string{"FOO": "a b", "BAR": "x'y"},
		Nonce:   "deadbeef",
	})
	for _, want := range []string{
		"__RHOST_DONE_deadbeef__:",
		"__RHOST_BEGIN_deadbeef__",
		"rhost-deadbeef.pid",
		"cd -- '/tmp'",
		"export BAR='x'\\''y'",
		"export FOO='a b'",
		"echo hi",
		"exit 0",
	} {
		if !strings.Contains(s, want) {
			t.Errorf("script missing %q:\n%s", want, s)
		}
	}
}

func TestParseMarker(t *testing.T) {
	const nonce = "abc123"
	marker := "\n__RHOST_DONE_" + nonce + "__:7\n"

	tests := []struct {
		name     string
		stdout   string
		wantBody string
		wantCode int
		wantOK   bool
	}{
		{"normal", "hi\n" + marker, "hi\n", 7, true},
		{"no trailing newline in output", "hi" + marker, "hi", 7, true},
		{"empty output", marker, "", 7, true},
		{"zero exit", "x\n\n__RHOST_DONE_" + nonce + "__:0\n", "x\n", 0, true},
		{
			"login profile noise before begin marker is dropped",
			"profile says hi\n\n__RHOST_BEGIN_" + nonce + "__\nreal output\n" + marker,
			"real output\n", 7, true,
		},
		{
			"fake marker with other nonce is ignored",
			"noise\n__RHOST_DONE_othernonce__:5\nreal\n" + marker,
			"noise\n__RHOST_DONE_othernonce__:5\nreal\n", 7, true,
		},
		{"truncated marker", "hi\n__RHOST_DONE_" + nonce + "__:7", "hi\n__RHOST_DONE_" + nonce + "__:7", -1, false},
		{"no marker", "just output\n", "just output\n", -1, false},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			body, code, ok := ParseMarker([]byte(tc.stdout), nonce)
			if ok != tc.wantOK {
				t.Fatalf("ok=%v want %v", ok, tc.wantOK)
			}
			if string(body) != tc.wantBody {
				t.Errorf("body=%q want %q", body, tc.wantBody)
			}
			if code != tc.wantCode {
				t.Errorf("code=%d want %d", code, tc.wantCode)
			}
		})
	}
}

func TestWrapScriptQuoteSafe(t *testing.T) {
	got := WrapScript("echo 'a b'")
	want := "exec setsid bash -lc 'eval \"$(printf %s ZWNobyAnYSBiJw== | base64 -d)\"'"
	if got != want {
		t.Errorf("WrapScript = %q, want %q", got, want)
	}
}

func TestKillCommandReferencesNonce(t *testing.T) {
	if c := KillCommand("xyz"); !strings.Contains(c, "rhost-xyz.pid") {
		t.Errorf("KillCommand missing pidfile name: %s", c)
	}
}

// TestKillCommandChecksBothPidLocations guards the state-dir-unwritable path:
// BuildScript records the pid under ${TMPDIR:-/tmp} when the state dir is not
// writable, so the killer must look there too or the remote process leaks.
func TestKillCommandChecksBothPidLocations(t *testing.T) {
	c := KillCommand("xyz")
	for _, want := range []string{
		"${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost}/run/rhost-xyz.pid",
		"${TMPDIR:-/tmp}/rhost-xyz.pid",
	} {
		if !strings.Contains(c, want) {
			t.Errorf("KillCommand missing %q: %s", want, c)
		}
	}
}
