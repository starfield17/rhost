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

// A command group invoked with no subcommand used to print its help text on stdout
// and exit 0. In --json mode that is human prose where an agent is parsing exactly
// one JSON document, so a bare group is reported as the usage error it is
// (AGENTS.md §6); a human still gets the help.
func TestBareGroupIsAUsageError(t *testing.T) {
	t.Cleanup(saveGlobals())

	for _, group := range []string{"session", "job", "fs"} {
		jsonFlag = true
		exitCode = 0
		os.Args = []string{"rhost", "--json", group}
		out := captureStdout(t, func() { Run() })

		var doc struct {
			OK        bool   `json:"ok"`
			Operation string `json:"operation"`
			Error     struct {
				Code string `json:"code"`
			} `json:"error"`
		}
		if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &doc); err != nil {
			t.Fatalf("%s: stdout is not one JSON document: %v\n%q", group, err, out)
		}
		if doc.OK || doc.Error.Code != "USAGE_ERROR" {
			t.Errorf("%s: bare group = %+v, want ok=false USAGE_ERROR", group, doc)
		}
		if doc.Operation != group+".usage" {
			t.Errorf("%s: operation = %q, want %q", group, doc.Operation, group+".usage")
		}
		if exitCode != 255 {
			t.Errorf("%s: exit = %d, want 255", group, exitCode)
		}
	}

	// The human path keeps the help text.
	jsonFlag = false
	exitCode = 0
	os.Args = []string{"rhost", "job"}
	out := captureStdout(t, func() { Run() })
	if !strings.Contains(out, "Usage:") || !strings.Contains(out, "start") {
		t.Errorf("bare group without --json must still print help, got %q", out)
	}
	if exitCode != 0 {
		t.Errorf("help exit = %d, want 0", exitCode)
	}
}

// TestNoCommandAtAllIsAUsageError is the root-level case of the rule above: the
// bare program used to print its help and exit 0 even under --json, which leaves an
// agent parsing prose as if it were a result document.
func TestNoCommandAtAllIsAUsageError(t *testing.T) {
	t.Cleanup(saveGlobals())

	jsonFlag = true
	exitCode = 0
	os.Args = []string{"rhost", "--json"}
	out := captureStdout(t, func() { Run() })

	var doc struct {
		OK    bool `json:"ok"`
		Error *struct {
			Code string `json:"code"`
		} `json:"error"`
	}
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &doc); err != nil {
		t.Fatalf("stdout is not one JSON document: %v\n%q", err, out)
	}
	if doc.OK || doc.Error == nil || doc.Error.Code != "USAGE_ERROR" {
		t.Errorf("bare rhost --json = %s, want ok=false USAGE_ERROR", out)
	}
	if exitCode != 255 {
		t.Errorf("exit = %d, want 255", exitCode)
	}

	// A human who typed nothing still gets the help, and still gets exit 0.
	jsonFlag = false
	exitCode = 0
	os.Args = []string{"rhost"}
	out = captureStdout(t, func() { Run() })
	if !strings.Contains(out, "Usage:") || !strings.Contains(out, "Available Commands") {
		t.Errorf("bare rhost without --json must print help, got %q", out)
	}
	if exitCode != 0 {
		t.Errorf("help exit = %d, want 0", exitCode)
	}
}

func TestDirectExecutionRequiresOneShellStringAfterDash(t *testing.T) {
	t.Cleanup(saveGlobals())
	for _, args := range [][]string{
		{"rhost", "--json", "--host", "example-host", "ls"},
		{"rhost", "--json", "--host", "example-host", "--", "ls", "-a"},
		{"rhost", "--json", "--host", "example-host", "--"},
	} {
		os.Args = args
		out := captureStdout(t, func() { _ = Run() })
		var doc struct {
			Error struct {
				Code string `json:"code"`
			} `json:"error"`
		}
		if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &doc); err != nil {
			t.Fatalf("%v: %v: %q", args, err, out)
		}
		if doc.Error.Code != "USAGE_ERROR" {
			t.Errorf("%v: code = %q", args, doc.Error.Code)
		}
	}
}

func TestDirectExecutionRejectsNegativeOutputLimit(t *testing.T) {
	t.Cleanup(saveGlobals())
	os.Args = []string{"rhost", "--json", "--host", "example-host", "--max-output-bytes", "-1", "--", "true"}
	out := captureStdout(t, func() { _ = Run() })
	var doc struct {
		Error struct {
			Code string `json:"code"`
		} `json:"error"`
	}
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &doc); err != nil || doc.Error.Code != "CONFIG_INVALID" {
		t.Fatalf("negative limit: expected CONFIG_INVALID, got %q (%v)", out, err)
	}
}

func TestRemovedCommandsAreUsageErrors(t *testing.T) {
	t.Cleanup(saveGlobals())
	for _, args := range [][]string{
		{"status"}, {"watch"}, {"exec-many"}, {"fs", "grep"}, {"fs", "glob"},
	} {
		os.Args = append([]string{"rhost", "--json"}, args...)
		out := captureStdout(t, func() { _ = Run() })
		var doc struct {
			Error struct {
				Code string `json:"code"`
			} `json:"error"`
		}
		if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &doc); err != nil || doc.Error.Code != "USAGE_ERROR" {
			t.Errorf("%v: expected USAGE_ERROR, got %q (%v)", args, out, err)
		}
	}
}

func TestSessionAttachRejectsJSONBeforeSSH(t *testing.T) {
	t.Cleanup(saveGlobals())
	os.Args = []string{"rhost", "--json", "session", "attach", "example-host", "dev"}
	out := captureStdout(t, func() { Run() })

	var doc struct {
		OK    bool `json:"ok"`
		Error struct {
			Code string `json:"code"`
		} `json:"error"`
	}
	if err := json.Unmarshal([]byte(strings.TrimSpace(out)), &doc); err != nil {
		t.Fatalf("attach --json must emit one envelope: %v\n%q", err, out)
	}
	if doc.OK || doc.Error.Code != "USAGE_ERROR" {
		t.Errorf("attach --json = %s, want USAGE_ERROR", out)
	}
}
