package app

import (
	"strings"
	"testing"
)

// TestLiveAudit proves the audit trail is real across processes: one rhost writes
// an entry for a remote operation, and a *separate* rhost reads it back.
func TestLiveAudit(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t).withEnv("RHOST_STATE_DIR="+t.TempDir(), "RHOST_AUDIT=1")

	c.mustJSON(t, "--json", "exec", host, "--command", "echo audited; exit 5")

	env := c.mustJSON(t, "--json", "audit")
	var entries []struct {
		Host      string `json:"host"`
		Operation string `json:"operation"`
		Command   string `json:"command_summary"`
		ExitCode  *int   `json:"exit_code"`
		OK        bool   `json:"ok"`
	}
	env.field(t, "entries", &entries)
	if len(entries) != 1 {
		t.Fatalf("want exactly 1 audited entry, got %d: %+v", len(entries), entries)
	}
	e := entries[0]
	if e.Operation != "exec" || e.Host != host {
		t.Errorf("entry = %+v, want operation=exec host=%s", e, host)
	}
	if e.ExitCode == nil || *e.ExitCode != 5 {
		t.Errorf("exit_code = %v, want 5", e.ExitCode)
	}
	if !strings.Contains(e.Command, "echo audited") {
		t.Errorf("command_summary = %q", e.Command)
	}
	if !e.OK {
		t.Error("ok = false for a completed operation")
	}
}
