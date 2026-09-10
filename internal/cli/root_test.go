package cli

import (
	"encoding/json"
	"io"
	"os"
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/errs"
)

// TestExitCodePolicy pins the contract documented in README/SKILL: a status in
// 0-254 belongs to the remote command, so every adapter failure must be 255 and
// a timeout 124 — uniformly, in every subcommand.
func TestExitCodePolicy(t *testing.T) {
	cases := []struct {
		code errs.Code
		want int
	}{
		{errs.SSHUnreachable, 255},
		{errs.SSHAuthFailed, 255},
		{errs.HostKeyFailed, 255},
		{errs.HostUnknown, 255},
		{errs.ConfigInvalid, 255},
		{errs.SessionNotFound, 255},
		{errs.SessionUnhealthy, 255},
		{errs.Internal, 255},
		{errs.RemoteCommandTimeout, 124},
	}
	for _, tc := range cases {
		if got := adapterExitCode(errs.New(tc.code, "x", false)); got != tc.want {
			t.Errorf("adapterExitCode(%s) = %d, want %d", tc.code, got, tc.want)
		}
	}
}

// TestEmitFailureJSONGoesToStdoutOnly enforces the rule that machine consumers
// get the envelope on stdout with no human noise mixed in.
func TestEmitFailureJSONGoesToStdoutOnly(t *testing.T) {
	t.Cleanup(saveGlobals())

	jsonFlag = true
	exitCode = 0
	out := captureStdout(t, func() {
		emitFailure("exec", "gpu", errs.New(errs.HostUnknown, "no such host", false))
	})

	if exitCode != 255 {
		t.Errorf("exitCode = %d, want 255", exitCode)
	}
	var doc map[string]interface{}
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &doc); err != nil {
		t.Fatalf("stdout is not one JSON document: %v\n%q", err, out)
	}
	if doc["ok"] != false || doc["schema_version"].(float64) != 1 {
		t.Errorf("unexpected envelope: %v", doc)
	}
	errObj := doc["error"].(map[string]interface{})
	if errObj["code"] != "HOST_UNKNOWN" {
		t.Errorf("error.code = %v, want HOST_UNKNOWN", errObj["code"])
	}
}

// TestUsageErrorStillHasACode is the AGENTS.md §6 guarantee for the one path
// that bypasses the application layer: bad flags/args must still be
// machine-readable, and must not masquerade as a remote exit status.
func TestUsageErrorStillHasACode(t *testing.T) {
	t.Cleanup(saveGlobals())

	os.Args = []string{"rhost", "--json", "exec"} // exec needs <host> plus a command
	code := 0
	out := captureStdout(t, func() { code = Run() })

	if code != 255 {
		t.Errorf("usage error exit = %d, want 255", code)
	}
	var doc struct {
		OK    bool `json:"ok"`
		Error struct {
			Code      string `json:"code"`
			Retryable bool   `json:"retryable"`
		} `json:"error"`
	}
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &doc); err != nil {
		t.Fatalf("usage error must emit an envelope on stdout: %v\n%q", err, out)
	}
	if doc.OK {
		t.Error("usage error envelope must have ok=false")
	}
	if doc.Error.Code != "USAGE_ERROR" {
		t.Errorf("usage error code = %q, want USAGE_ERROR", doc.Error.Code)
	}
	if doc.Error.Retryable {
		t.Error("a usage error is never retryable")
	}
}

// TestUsageErrorHumanPathKeepsStderrClean checks the non-JSON form stays on
// stderr so that stdout remains pipeable.
func TestUsageErrorHumanPathKeepsStderrClean(t *testing.T) {
	t.Cleanup(saveGlobals())

	os.Args = []string{"rhost", "exec"}
	jsonFlag = false
	code := 0
	out := captureStdout(t, func() { code = Run() })

	if code != 255 {
		t.Errorf("usage error exit = %d, want 255", code)
	}
	if out != "" {
		t.Errorf("human usage error must not write to stdout, got %q", out)
	}
}

func saveGlobals() func() {
	oldJSON, oldExit, oldArgs := jsonFlag, exitCode, os.Args
	return func() {
		jsonFlag, exitCode, os.Args = oldJSON, oldExit, oldArgs
	}
}

// captureStdout runs fn with os.Stdout redirected to a pipe and returns what was
// written.
func captureStdout(t *testing.T, fn func()) string {
	t.Helper()

	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	old := os.Stdout
	os.Stdout = w
	defer func() { os.Stdout = old }()

	done := make(chan string, 1)
	go func() {
		b, _ := io.ReadAll(r)
		done <- string(b)
	}()

	fn()

	if err := w.Close(); err != nil {
		t.Fatal(err)
	}
	got := <-done
	if err := r.Close(); err != nil {
		t.Fatal(err)
	}
	return got
}
