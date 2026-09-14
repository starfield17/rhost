package conformance

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestSmokeLive covers one representative path through each core workflow.
// It deliberately stays small enough for frequent pre-push use; the TestLive*
// suites retain the exhaustive persistence, failure and transport coverage.
func TestSmokeLive(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	doctor := c.mustJSON(t, "--json", "doctor", host)
	var capabilities map[string]bool
	doctor.field(t, "capabilities", &capabilities)
	if !capabilities["bash"] || !capabilities["tmux"] {
		t.Fatalf("smoke test needs bash and tmux: %v", capabilities)
	}

	direct := c.mustJSON(t, "--json", "exec", host, "--command", "printf smoke-exec")
	if direct.str(t, "stdout") != "smoke-exec" || direct.num(t, "exit_code") != 0 {
		t.Fatalf("direct exec contract: %s", direct.Data)
	}

	dir := remoteTemp(t, c, host)
	local := filepath.Join(t.TempDir(), "input.txt")
	if err := os.WriteFile(local, []byte("smoke-file\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	remote := dir + "/nested/input.txt"
	written := c.mustJSON(t, "--json", "fs", "write", host, remote, "--from", local, "--parents")
	read := c.mustJSON(t, "--json", "fs", "read", host, remote)
	if read.str(t, "content") != "smoke-file\n" || read.str(t, "sha256") != written.str(t, "sha256") {
		t.Fatalf("file read/write contract: write=%s read=%s", written.Data, read.Data)
	}
	c.wantErrorCode(t, "FILE_CONFLICT", "--json", "fs", "write", host, remote,
		"--from", local, "--if-hash", strings.Repeat("0", 64))

	session := liveName("smoke-session")
	created := c.mustJSON(t, "--json", "session", "create", host, "--name", session)
	if created.str(t, "session_id") == "" {
		t.Fatal("session.create returned no session_id")
	}
	defer c.cleanup(t, "--json", "session", "close", host, session)
	c.mustJSON(t, "--json", "session", "exec", host, session, "--command", "cd /tmp; export RHOST_SMOKE_VALUE=42")
	preserved := c.mustJSON(t, "--json", "session", "exec", host, session, "--command", "printf '%s|%s' \"$PWD\" \"$RHOST_SMOKE_VALUE\"")
	if !strings.Contains(preserved.str(t, "stdout"), "/tmp|42") {
		t.Fatalf("session state did not cross CLI processes: %s", preserved.Data)
	}
}
