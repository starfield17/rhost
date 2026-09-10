package openssh

import (
	"context"
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
	joined := strings.Join(New(DefaultConfig()).options(), " ")
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

// TestRunMissingSSHBinary covers the only path where Run returns a Go error:
// the ssh binary could not be started at all.
func TestRunMissingSSHBinary(t *testing.T) {
	c := New(Config{SSHBin: "/nonexistent/rhost-ssh-binary"})
	_, err := c.Run(context.Background(), "gpu", "true", time.Second)
	if err == nil {
		t.Fatal("Run should error when ssh cannot be started")
	}
}
