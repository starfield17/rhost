package openssh

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestDefaultConfig(t *testing.T) {
	t.Setenv("RHOST_SSH_LOG_LEVEL", "")
	cfg := DefaultConfig()

	if !strings.HasSuffix(cfg.ControlPath, "%C") {
		t.Errorf("ControlPath = %q, want a %%C template so each target gets its own socket", cfg.ControlPath)
	}
	if !cfg.BatchMode {
		t.Error("BatchMode must default on: agents must never be prompted for a password")
	}
	if cfg.ConnectTimeout != 15*time.Second {
		t.Errorf("ConnectTimeout = %v, want 15s", cfg.ConnectTimeout)
	}
	if cfg.LogLevel != "ERROR" {
		t.Errorf("LogLevel = %q, want ERROR (ssh must stay silent on success)", cfg.LogLevel)
	}
}

// TestLogLevelIsOverridable pins the escape hatch for diagnosing transport
// failures: OpenSSH suppresses its own reason at LogLevel=ERROR, so operators
// need a way to raise it without rebuilding.
func TestLogLevelIsOverridable(t *testing.T) {
	t.Setenv("RHOST_SSH_LOG_LEVEL", "VERBOSE")
	if got := DefaultConfig().LogLevel; got != "VERBOSE" {
		t.Errorf("LogLevel = %q, want VERBOSE from RHOST_SSH_LOG_LEVEL", got)
	}
}

func TestNewFillsZeroValues(t *testing.T) {
	c := New(Config{LogLevel: "QUIET"})
	cfg := c.Config()
	if cfg.SSHBin != "ssh" {
		t.Errorf("SSHBin = %q, want ssh", cfg.SSHBin)
	}
	if cfg.ControlPath == "" || !strings.Contains(cfg.ControlPath, "rhost") {
		t.Errorf("ControlPath = %q, want rhost's own namespace", cfg.ControlPath)
	}
	if cfg.ControlPersist == "" || cfg.ConnectTimeout <= 0 {
		t.Errorf("zero values not filled: %+v", cfg)
	}
	if cfg.LogLevel != "QUIET" {
		t.Errorf("explicit LogLevel must survive: %q", cfg.LogLevel)
	}
}

// TestOptionsIsolatesTheControlMasterNamespace documents the persistence
// invariant: reuse lives in OpenSSH's socket directory, not in this process.
func TestOptions(t *testing.T) {
	joined := strings.Join(New(DefaultConfig()).options(false), " ")
	for _, want := range []string{
		"-o ControlMaster=auto",
		"-o ControlPersist=15m",
		"-o ControlPath=",
		"-o ConnectTimeout=15",
		"-o BatchMode=yes",
	} {
		if !strings.Contains(joined, want) {
			t.Errorf("options missing %q in: %s", want, joined)
		}
	}
	for _, banned := range []string{"StrictHostKeyChecking=no", "UserKnownHostsFile=/dev/null"} {
		if strings.Contains(joined, banned) {
			t.Errorf("options must never disable host-key checking: %s", banned)
		}
	}
}

func TestFreshOptionsDisableAllMultiplexing(t *testing.T) {
	joined := strings.Join(New(DefaultConfig()).options(true), " ")
	for _, want := range []string{"ControlMaster=no", "ControlPersist=no", "ControlPath=none"} {
		if !strings.Contains(joined, want) {
			t.Errorf("fresh options missing %q in %s", want, joined)
		}
	}
	if strings.Contains(joined, "ControlMaster=auto") {
		t.Fatalf("fresh options still allow reuse: %s", joined)
	}
}

func TestConnectionStatusAndReset(t *testing.T) {
	dir := t.TempDir()
	logPath := filepath.Join(dir, "calls")
	ssh := filepath.Join(dir, "ssh")
	script := `#!/bin/sh
printf '%s\n' "$*" >> "$RHOST_TEST_CALLS"
case "$*" in
  *"-O check"*) echo 'Master running (pid=4321)' >&2; exit 0 ;;
  *"-O stop"*) echo 'Stop listening request sent.' >&2; exit 0 ;;
esac
exit 2
`
	if err := os.WriteFile(ssh, []byte(script), 0o700); err != nil {
		t.Fatal(err)
	}
	t.Setenv("RHOST_TEST_CALLS", logPath)
	c := New(Config{SSHBin: ssh, ControlPath: filepath.Join(dir, "%C")})
	status := c.ConnectionStatus(context.Background(), "example-host")
	if status.MasterStatus != MasterAlive || status.MasterPID != 4321 {
		t.Fatalf("status = %+v", status)
	}
	reset, err := c.ResetConnection(context.Background(), "example-host")
	if err != nil || !reset.Stopped {
		t.Fatalf("reset = %+v err=%v", reset, err)
	}
	calls, err := os.ReadFile(logPath)
	if err != nil || !strings.Contains(string(calls), "-O stop") {
		t.Fatalf("calls = %q err=%v", calls, err)
	}
}

func TestConnectionStatusDistinguishesAbsentAndUnknown(t *testing.T) {
	for _, tc := range []struct {
		name, diagnostic string
		want             MasterStatus
	}{
		{"absent", "Control socket connect(/tmp/rhost): No such file or directory", MasterAbsent},
		{"permission", "Control socket connect(/tmp/rhost): Permission denied", MasterUnknown},
	} {
		t.Run(tc.name, func(t *testing.T) {
			dir := t.TempDir()
			ssh := filepath.Join(dir, "ssh")
			script := "#!/bin/sh\necho '" + tc.diagnostic + "' >&2\nexit 255\n"
			if err := os.WriteFile(ssh, []byte(script), 0o700); err != nil {
				t.Fatal(err)
			}
			c := New(Config{SSHBin: ssh, ControlPath: filepath.Join(dir, "%C")})
			if got := c.ConnectionStatus(context.Background(), "example-host"); got.MasterStatus != tc.want {
				t.Fatalf("status = %+v, want %s", got, tc.want)
			}
			if tc.want == MasterUnknown {
				if _, err := c.ResetConnection(context.Background(), "example-host"); err == nil {
					t.Fatal("reset treated an unknown master as stopped")
				}
			}
		})
	}
}

func TestFreshRunSkipsSharedControlDirectory(t *testing.T) {
	dir := t.TempDir()
	blocked := filepath.Join(dir, "not-a-directory")
	if err := os.WriteFile(blocked, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("RHOST_CACHE_DIR", blocked)
	c := New(Config{SSHBin: "/usr/bin/true"})
	if status := c.ConnectionStatus(context.Background(), "example-host"); status.MasterStatus != MasterAlive {
		t.Fatalf("status check touched shared control state: %+v", status)
	}
	if _, err := c.RunWith(context.Background(), "example-host", "true", RunOptions{Fresh: true}); err != nil {
		t.Fatalf("fresh run touched shared control state: %v", err)
	}
	if _, err := c.RunWith(context.Background(), "example-host", "true", RunOptions{}); err == nil {
		t.Fatal("shared run unexpectedly ignored its unsafe control directory")
	}
}

// TestRunMissingSSHBinary covers the only path where Run returns a Go error:
// the ssh binary could not be started at all.
func TestRunMissingSSHBinary(t *testing.T) {
	c := New(Config{SSHBin: "/nonexistent/rhost-ssh-binary"})
	_, err := c.Run(context.Background(), "gpu", "true", time.Second)
	if err == nil {
		t.Fatal("Run should error when ssh cannot be started")
	}
}
