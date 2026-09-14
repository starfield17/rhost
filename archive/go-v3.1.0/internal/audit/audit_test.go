package audit

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"unicode/utf8"
)

func TestRecordAppendsJSONLines(t *testing.T) {
	p := Path(t.TempDir())
	r := &Recorder{Path: p}
	code := 3
	for i, e := range []Entry{
		{Time: "2026-01-01T00:00:00Z", Host: "gpu", Operation: "exec", Command: "pytest -q", ExitCode: &code, DurationMS: 831, OK: false, ErrorCode: "TRANSFER_FAILED"},
		{Time: "2026-01-01T00:01:00Z", Host: "gpu", Operation: "doctor", OK: true},
	} {
		if err := r.Record(e); err != nil {
			t.Fatalf("record %d: %v", i, err)
		}
	}

	raw, err := os.ReadFile(p)
	if err != nil {
		t.Fatalf("read audit log: %v", err)
	}
	lines := strings.Split(strings.TrimRight(string(raw), "\n"), "\n")
	if len(lines) != 2 {
		t.Fatalf("got %d lines, want 2:\n%s", len(lines), raw)
	}

	var first Entry
	if err := json.Unmarshal([]byte(lines[0]), &first); err != nil {
		t.Fatalf("first line is not JSON: %v", err)
	}
	if first.Host != "gpu" || first.Operation != "exec" || first.Command != "pytest -q" {
		t.Errorf("round trip lost fields: %+v", first)
	}
	if first.ExitCode == nil || *first.ExitCode != 3 || first.DurationMS != 831 || first.OK {
		t.Errorf("round trip lost detail: %+v", first)
	}
}

// A disabled auditor must not create anything, and must not require the caller to
// check a flag.
func TestDisabledRecorderWritesNothing(t *testing.T) {
	dir := t.TempDir()
	var nilRec *Recorder
	if err := nilRec.Record(Entry{Operation: "exec"}); err != nil {
		t.Errorf("nil recorder: %v", err)
	}
	if err := (&Recorder{}).Record(Entry{Operation: "exec"}); err != nil {
		t.Errorf("empty path: %v", err)
	}
	if entries, _ := os.ReadDir(dir); len(entries) != 0 {
		t.Errorf("disabled recorder wrote %v", entries)
	}
}

func TestEnabledHonoursEnv(t *testing.T) {
	for _, off := range []string{"0", "false", "no", "off", "FALSE", " Off "} {
		t.Setenv(EnvVar, off)
		if Enabled() {
			t.Errorf("%s=%q must disable auditing", EnvVar, off)
		}
	}
	for _, on := range []string{"", "1", "true", "yes", "anything"} {
		t.Setenv(EnvVar, on)
		if !Enabled() {
			t.Errorf("%s=%q must enable auditing", EnvVar, on)
		}
	}
}

// The audit file and its directory must be user-private (§35).
func TestRecordUsesPrivatePermissions(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "state")
	p := Path(dir)
	if err := os.Mkdir(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, nil, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := (&Recorder{Path: p}).Record(Entry{Operation: "exec"}); err != nil {
		t.Fatal(err)
	}
	fi, err := os.Stat(p)
	if err != nil {
		t.Fatal(err)
	}
	if fi.Mode().Perm() != 0o600 {
		t.Errorf("audit file mode = %v, want 0600", fi.Mode().Perm())
	}
	di, err := os.Stat(dir)
	if err != nil {
		t.Fatal(err)
	}
	if di.Mode().Perm() != 0o700 {
		t.Errorf("state dir mode = %v, want 0700", di.Mode().Perm())
	}
}

// Fail-open is the caller's job, but Record must report the failure rather than
// panic or silently succeed: a path under a regular file cannot be created.
func TestRecordReportsFailure(t *testing.T) {
	blocker := filepath.Join(t.TempDir(), "not-a-dir")
	if err := os.WriteFile(blocker, []byte("x"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := (&Recorder{Path: filepath.Join(blocker, "audit.jsonl")}).Record(Entry{Operation: "exec"}); err == nil {
		t.Error("writing under a regular file must fail, not vanish")
	}
}

func TestSummarize(t *testing.T) {
	got := Summarize("line one\nline two\n")
	if got != "line one line two" {
		t.Errorf("Summarize collapsed to %q", got)
	}

	long := strings.Repeat("a", maxCommand+50)
	got = Summarize(long)
	if !strings.HasSuffix(got, "…") || utf8.RuneCountInString(got) != maxCommand+1 {
		t.Errorf("Summarize did not truncate: runes=%d", utf8.RuneCountInString(got))
	}

	// Truncation must not split a multi-byte rune: put a 2-byte rune across the
	// 200-byte boundary, so a naive s[:200] would cut it in half.
	multibyte := strings.Repeat("a", maxCommand-1) + "é" + strings.Repeat("a", 50)
	got = Summarize(multibyte)
	if !utf8.ValidString(got) {
		t.Errorf("Summarize produced invalid UTF-8: %q", got)
	}
}
