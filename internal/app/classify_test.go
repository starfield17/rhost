package app

import (
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// classifyMissingMarker turns "no completion marker" runs into taxonomy codes.
// Agents branch on Code, so the mapping is a contract and is pinned here.
func TestClassifyMissingMarker(t *testing.T) {
	cases := []struct {
		name        string
		stderr      string
		exitCode    int
		wantCode    errs.Code
		wantRetry   bool
		wantMessage string // substring
	}{
		{
			name:        "remote dependency missing",
			stderr:      "bash: line 1: setsid: command not found",
			exitCode:    127,
			wantCode:    errs.RemoteDependencyMissing,
			wantRetry:   false,
			wantMessage: "setsid",
		},
		{
			// A long cache dir used to make every command fail opaquely.
			name:        "control path too long is a config fault",
			stderr:      "ControlPath too long ('/var/folders/x/.../rhost/ssh/abc' >= 104 bytes)",
			exitCode:    255,
			wantCode:    errs.ConfigInvalid,
			wantRetry:   false,
			wantMessage: "ControlPath too long",
		},
		{
			name:        "auth failure is not retryable",
			stderr:      "git@build.example.internal: Permission denied (publickey,password).",
			exitCode:    255,
			wantCode:    errs.SSHAuthFailed,
			wantRetry:   false,
			wantMessage: "Permission denied",
		},
		{
			name:        "host key changed",
			stderr:      "@@@@@@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED\nHost key verification failed.",
			exitCode:    255,
			wantCode:    errs.HostKeyFailed,
			wantRetry:   false,
			wantMessage: "host key verification failed",
		},
		{
			name:        "unresolvable host",
			stderr:      "ssh: Could not resolve hostname gpu: nodename nor servname provided",
			exitCode:    255,
			wantCode:    errs.HostUnknown,
			wantRetry:   false,
			wantMessage: "Could not resolve hostname",
		},
		{
			name:        "connection refused",
			stderr:      "ssh: connect to host build.example.internal port 22: Connection refused",
			exitCode:    255,
			wantCode:    errs.SSHUnreachable,
			wantRetry:   true,
			wantMessage: "connection refused",
		},
		{
			name:        "connection reset",
			stderr:      "Read from remote host build.example.internal: Connection reset by peer",
			exitCode:    255,
			wantCode:    errs.RemoteExecutionUnknown,
			wantRetry:   false,
			wantMessage: "Connection reset",
		},
		{
			name:        "timeout during banner",
			stderr:      "Connection timed out during banner exchange",
			exitCode:    255,
			wantCode:    errs.RemoteExecutionUnknown,
			wantRetry:   false,
			wantMessage: "connection timed out",
		},
		{
			// OpenSSH suppresses its own diagnostics at LogLevel=ERROR, so this
			// empty-stderr shape is a real failure mode, not a corner case.
			name:        "silent ssh failure points at the verbosity switch",
			stderr:      "",
			exitCode:    255,
			wantCode:    errs.RemoteExecutionUnknown,
			wantRetry:   false,
			wantMessage: "RHOST_SSH_LOG_LEVEL=VERBOSE",
		},
		{
			name:        "command exited non-zero without a marker",
			stderr:      "boom\nsecond line\n",
			exitCode:    1,
			wantCode:    errs.SSHUnreachable,
			wantRetry:   true,
			wantMessage: "boom",
		},
		{
			name:        "clean exit but no marker is an internal fault",
			stderr:      "",
			exitCode:    0,
			wantCode:    errs.Internal,
			wantRetry:   false,
			wantMessage: "no completion marker",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := classifyMissingMarker(openssh.Result{Stderr: []byte(tc.stderr), ExitCode: tc.exitCode})
			if got == nil {
				t.Fatal("classifyMissingMarker returned nil, want an error")
			}
			if got.Code != tc.wantCode {
				t.Errorf("Code = %s, want %s (message: %s)", got.Code, tc.wantCode, got.Message)
			}
			if got.Retryable != tc.wantRetry {
				t.Errorf("Retryable = %v, want %v", got.Retryable, tc.wantRetry)
			}
			if !strings.Contains(got.Message, tc.wantMessage) {
				t.Errorf("Message = %q, want substring %q", got.Message, tc.wantMessage)
			}
			if strings.Contains(got.Message, "\n") {
				t.Errorf("Message should be one line, got %q", got.Message)
			}
		})
	}
}

// TestClassifyDoesNotInventACode guards the fallback: an unrecognised ssh
// diagnostic must surface as SSH_UNREACHABLE with its own text, not silently
// become INTERNAL or an auth/host-key verdict.
func TestClassifySSHUnknownDiagnosticIsExecutionUnknown(t *testing.T) {
	got := classifyMissingMarker(openssh.Result{Stderr: []byte("some brand new ssh complaint"), ExitCode: 255})
	if got.Code != errs.RemoteExecutionUnknown {
		t.Errorf("Code = %s, want %s", got.Code, errs.RemoteExecutionUnknown)
	}
	if got.Message != "some brand new ssh complaint" {
		t.Errorf("Message = %q, want the first stderr line verbatim", got.Message)
	}
}

func TestClassifyRemotePermissionDeniedIsNotSSHAuth(t *testing.T) {
	got := classifyMissingMarker(openssh.Result{Stderr: []byte("mkdir: Permission denied"), ExitCode: 1})
	if got.Code != errs.RemoteExecutionUnknown || got.Retryable {
		t.Fatalf("classification = %s retryable=%v", got.Code, got.Retryable)
	}
}

func TestClassifyMarkerMissingPrefersDependencyOverSSH(t *testing.T) {
	// A remote without bash/setsid also prints ssh noise; the actionable code is
	// the missing dependency.
	stderr := "Warning: Permanently added host\nbash: setsid: command not found\n"
	got := classifyMissingMarker(openssh.Result{Stderr: []byte(stderr), ExitCode: 127})
	if got.Code != errs.RemoteDependencyMissing {
		t.Errorf("Code = %s, want %s", got.Code, errs.RemoteDependencyMissing)
	}
}

func TestFirstLine(t *testing.T) {
	cases := map[string]string{
		"":            "",
		"  \n":        "",
		"one":         "one",
		"one\ntwo":    "one",
		"\n  lead\ne": "lead",
	}
	for in, want := range cases {
		if got := firstLine(in); got != want {
			t.Errorf("firstLine(%q) = %q, want %q", in, got, want)
		}
	}
}
