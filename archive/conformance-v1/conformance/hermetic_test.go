package conformance

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

// These stubs stop at the OpenSSH boundary. They never parse, emit, or depend
// on either implementation's private completion protocol.
func TestConformanceExecUncertainty(t *testing.T) {
	bin := buildBinary(t)
	for _, tc := range []struct {
		name, diagnostic, code string
		retry                  bool
	}{
		{"silent-channel-loss", "", "REMOTE_EXECUTION_UNKNOWN", false},
		{"reset-after-submission", "Read from remote host example-host: Connection reset by peer", "REMOTE_EXECUTION_UNKNOWN", false},
		{"authentication", "Permission denied (publickey).", "SSH_AUTH_FAILED", false},
		{"host-key", "Host key verification failed.", "HOST_KEY_FAILED", false},
		{"connection-refused", "ssh: connect to host example-host port 22: Connection refused", "SSH_UNREACHABLE", true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			script := "#!/bin/sh\nprintf '%s\\n' " + quoteShell(tc.diagnostic) + " >&2\nexit 255\n"
			if err := os.WriteFile(filepath.Join(dir, "ssh"), []byte(script), 0700); err != nil {
				t.Fatal(err)
			}
			c := liveCLI{bin: bin}.withEnv("PATH="+dir+":"+os.Getenv("PATH"), "RHOST_CACHE_DIR="+t.TempDir(), "RHOST_STATE_DIR="+t.TempDir())
			e := c.wantErrorCode(t, tc.code, "exec", "gpu", "--fresh", "--json", "--command", "printf side-effect")
			if e.Error.Retryable != tc.retry {
				t.Fatalf("retryable=%v want %v", e.Error.Retryable, tc.retry)
			}
		})
	}
}

func TestConformanceOutputDeliveryFailure(t *testing.T) {
	sink, err := os.Open(os.DevNull)
	if err != nil {
		t.Fatal(err)
	}
	defer sink.Close()
	cmd := exec.Command(buildBinary(t), "version", "--json")
	cmd.Stdout = sink
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	err = cmd.Run()
	var status *exec.ExitError
	if !errors.As(err, &status) || status.ExitCode() != 255 {
		t.Fatalf("failed output delivery exit=%v", err)
	}
	if !bytes.Contains(stderr.Bytes(), []byte("OUTPUT_WRITE_FAILED")) {
		t.Fatalf("missing delivery error: %s", stderr.Bytes())
	}
}

func TestConformanceV1UnknownIsNotZero(t *testing.T) {
	raw := []byte(`{"schema_version":1,"operation":"exec","ok":false,"data":{"exit_code":-1,"stdout":"","stderr":"","timed_out":false,"cancelled":false,"cleanup_confirmed":false,"stdout_truncated":false,"stderr_truncated":false,"stdout_bytes":0,"stderr_bytes":0,"duration_ms":0},"error":{"code":"REMOTE_EXECUTION_UNKNOWN","message":"unknown","retryable":false}}`)
	var e envelope
	if err := json.Unmarshal(raw, &e); err != nil {
		t.Fatal(err)
	}
	if e.num(t, "exit_code") != -1 {
		t.Fatal("v1 unknown exit was normalized to success")
	}
}

// HIST-PROC-001: a forked tool descendant must not keep the CLI waiting on pipes.
func TestConformanceTransferDescendantTimeout(t *testing.T) {
	dir := t.TempDir()
	ready := filepath.Join(dir, "descendant-started")
	if err := os.WriteFile(filepath.Join(dir, "scp"), []byte("#!/bin/sh\nsleep 5 &\nprintf started > \"$RHOST_TEST_DESCENDANT_STARTED\"\nwait\n"), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "ssh"), []byte("#!/bin/sh\nexit 255\n"), 0700); err != nil {
		t.Fatal(err)
	}
	source := filepath.Join(t.TempDir(), "input")
	if err := os.WriteFile(source, []byte("input"), 0600); err != nil {
		t.Fatal(err)
	}
	c := liveCLI{bin: buildBinary(t)}.withEnv("RHOST_TEST_DESCENDANT_STARTED="+ready, "PATH="+dir+":"+os.Getenv("PATH"), "RHOST_CACHE_DIR="+t.TempDir(), "RHOST_STATE_DIR="+t.TempDir())
	started := time.Now()
	e := c.wantErrorCode(t, "REMOTE_COMMAND_TIMEOUT", "fs", "get", "gpu", "/tmp/rhost-transfer-test", source, "--json", "--timeout", "1s")
	if time.Since(started) > 3*time.Second {
		t.Fatal("forked descendant kept transfer pipes open beyond the cleanup budget")
	}
	if _, err := os.Stat(ready); err != nil {
		t.Fatal("timeout fixture never started its pipe-holding descendant", err)
	}
	if e.Error.Retryable {
		t.Fatal("timed-out transfer was marked safe to retry")
	}
}

func TestConformanceConnectionStatusReset(t *testing.T) {
	dir := t.TempDir()
	log := filepath.Join(dir, "calls")
	script := `#!/bin/sh
printf '%s\n' "$*" >> "$RHOST_TEST_CALLS"
case "$RHOST_TEST_MASTER_MODE" in
 absent) echo 'Control socket connect(/tmp/rhost-test): No such file or directory' >&2; exit 255 ;;
 unknown) echo 'Control socket connect(/tmp/rhost-test): Permission denied' >&2; exit 255 ;;
esac
case "$*" in
 *"-O check"*) echo 'Master running (pid=4321)' >&2; exit 0 ;;
 *"-O stop"*) echo 'Stop listening request sent.' >&2; exit 0 ;;
esac
exit 2
`
	if err := os.WriteFile(filepath.Join(dir, "ssh"), []byte(script), 0700); err != nil {
		t.Fatal(err)
	}
	c := liveCLI{bin: buildBinary(t)}.withEnv("PATH="+dir+":"+os.Getenv("PATH"), "RHOST_CACHE_DIR="+t.TempDir(), "RHOST_STATE_DIR="+t.TempDir(), "RHOST_TEST_CALLS="+log)
	status := c.mustJSON(t, "connection", "status", "gpu", "--json")
	if status.str(t, "master_status") != "alive" || status.num(t, "master_pid") != 4321 {
		t.Fatal("live master was not exposed")
	}
	reset := c.mustJSON(t, "connection", "reset", "gpu", "--json")
	if !reset.bool(t, "stopped") {
		t.Fatal("reset did not report stop")
	}
	calls, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(calls, []byte("-O stop")) || bytes.Contains(calls, []byte("-O exit")) {
		t.Fatalf("reset must stop accepting channels, not terminate them: %s", calls)
	}
	for _, mode := range []string{"absent", "unknown"} {
		isolated := c.withEnv("RHOST_TEST_MASTER_MODE=" + mode)
		if got := isolated.mustJSON(t, "connection", "status", "gpu", "--json").str(t, "master_status"); got != mode {
			t.Errorf("status=%s want %s", got, mode)
		}
		if mode == "unknown" {
			isolated.wantErrorCode(t, "SSH_CONTROL_FAILED", "connection", "reset", "gpu", "--json")
		}
	}
}

func TestConformanceMissingCompletionEvenWithZeroSSHStatus(t *testing.T) {
	c := liveCLI{bin: buildBinary(t)}.withEnv("RHOST_CACHE_DIR="+t.TempDir(), "RHOST_STATE_DIR="+t.TempDir())
	version := c.mustJSON(t, "version", "--json").SchemaVersion
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "ssh"), []byte("#!/bin/sh\nexit 0\n"), 0700); err != nil {
		t.Fatal(err)
	}
	c = c.withEnv("PATH=" + dir + ":" + os.Getenv("PATH"))
	code := "REMOTE_EXECUTION_UNKNOWN"
	if version == 1 {
		code = "INTERNAL"
	}
	e := c.wantErrorCode(t, code, "exec", "gpu", "--fresh", "--json", "--command", "true")
	if e.Error.Retryable {
		t.Fatal("missing completion became retryable")
	}
}
