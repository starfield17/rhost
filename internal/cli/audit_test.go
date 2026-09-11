package cli

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/audit"
)

// The CLI-side audit timer and reader, pinned without a host.

func TestAuditTimerWritesOneEntry(t *testing.T) {
	state := t.TempDir()
	t.Setenv("RHOST_STATE_DIR", state)
	t.Setenv("RHOST_AUDIT", "1")

	code := 7
	startAudit("exec", "gpu").succeed("/tmp", "echo hi; exit 7", &code)

	raw, err := os.ReadFile(audit.Path(state))
	if err != nil {
		t.Fatalf("audit log was not written: %v", err)
	}
	var e audit.Entry
	if err := json.Unmarshal(bytes.TrimSpace(raw), &e); err != nil {
		t.Fatalf("entry is not JSON: %v", err)
	}
	if e.Operation != "exec" || e.Host != "gpu" || e.Cwd != "/tmp" || e.Command != "echo hi; exit 7" {
		t.Errorf("entry = %+v", e)
	}
	if e.ExitCode == nil || *e.ExitCode != 7 || !e.OK || e.Time == "" {
		t.Errorf("entry lost detail: %+v", e)
	}
}

func TestAuditTimerRecordsFailure(t *testing.T) {
	state := t.TempDir()
	t.Setenv("RHOST_STATE_DIR", state)
	t.Setenv("RHOST_AUDIT", "1")

	startAudit("status", "gpu").fail(nil) // nil error must not panic

	raw, err := os.ReadFile(audit.Path(state))
	if err != nil {
		t.Fatal(err)
	}
	var e audit.Entry
	if err := json.Unmarshal(bytes.TrimSpace(raw), &e); err != nil {
		t.Fatal(err)
	}
	if e.OK || e.Operation != "status" {
		t.Errorf("failure entry = %+v", e)
	}
}

func TestAuditTimerDisabledWritesNothing(t *testing.T) {
	state := t.TempDir()
	t.Setenv("RHOST_STATE_DIR", state)
	t.Setenv("RHOST_AUDIT", "0")

	startAudit("exec", "gpu").succeed("", "true", nil)

	if _, err := os.Stat(audit.Path(state)); !os.IsNotExist(err) {
		t.Errorf("RHOST_AUDIT=0 must not write a log (err=%v)", err)
	}
}

func TestReadAuditLogSkipsCorruptLines(t *testing.T) {
	p := filepath.Join(t.TempDir(), "audit.jsonl")
	body := strings.Join([]string{
		`{"time":"t","host":"gpu","operation":"exec","ok":true}`,
		``,
		`not json at all`,
		`{"time":"t2","host":"other","operation":"status","ok":true}`,
	}, "\n")
	if err := os.WriteFile(p, []byte(body), 0o600); err != nil {
		t.Fatal(err)
	}
	entries, aerr := readAuditLog(p)
	if aerr != nil {
		t.Fatalf("readAuditLog: %v", aerr)
	}
	if len(entries) != 2 {
		t.Fatalf("got %d entries, want 2 (blank + corrupt skipped): %+v", len(entries), entries)
	}
	if got := filterAuditHost(entries, "other"); len(got) != 1 || got[0].Operation != "status" {
		t.Errorf("filterAuditHost = %+v", got)
	}
}

func TestReadAuditLogMissingIsEmpty(t *testing.T) {
	entries, aerr := readAuditLog(filepath.Join(t.TempDir(), "nope.jsonl"))
	if aerr != nil || len(entries) != 0 {
		t.Errorf("a missing log is an empty log, got %+v / %v", entries, aerr)
	}
}

func TestRenderAuditEmpty(t *testing.T) {
	var buf bytes.Buffer
	renderAudit(&buf, "/state/audit.jsonl", nil)
	if !strings.Contains(buf.String(), "no audit entries") {
		t.Errorf("empty audit render = %q", buf.String())
	}
}
