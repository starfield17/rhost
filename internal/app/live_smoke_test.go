package app

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/errs"
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

	direct := c.mustJSON(t, "--json", "--host", host, "--", "printf smoke-exec")
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
	c.wantErrorCode(t, errs.FileConflict, "--json", "fs", "write", host, remote,
		"--from", local, "--if-hash", strings.Repeat("0", 64))

	session := liveName("smoke-session")
	created := c.mustJSON(t, "--json", "session", "create", host, "--name", session)
	if created.str(t, "session_id") == "" {
		t.Fatal("session.create returned no session_id")
	}
	defer c.run(t, "--json", "session", "close", host, session)
	c.mustJSON(t, "--json", "session", "exec", host, session, "--", "cd /tmp; export RHOST_SMOKE_VALUE=42")
	preserved := c.mustJSON(t, "--json", "session", "exec", host, session, "--", "printf '%s|%s' \"$PWD\" \"$RHOST_SMOKE_VALUE\"")
	if !strings.Contains(preserved.str(t, "stdout"), "/tmp|42") {
		t.Fatalf("session state did not cross CLI processes: %s", preserved.Data)
	}

	started := c.mustJSON(t, "--json", "job", "start", host, "--", "printf smoke-job; sleep 60")
	jobID := started.str(t, "job_id")
	if jobID == "" {
		t.Fatal("job.start returned no job_id")
	}
	defer cleanupJob(t, c, host, jobID)
	status := c.mustJSON(t, "--json", "job", "status", host, jobID)
	var exitCode *int
	status.field(t, "exit_code", &exitCode)
	if status.str(t, "state") != "running" || exitCode != nil {
		t.Fatalf("running job status contract: %s", status.Data)
	}
	logs := c.mustJSON(t, "--json", "job", "logs", host, jobID, "--since", "0")
	if logs.str(t, "encoding") != "base64" || !strings.Contains(decodeLog(t, logs), "smoke-job") {
		t.Fatalf("job log contract: %s", logs.Data)
	}
	stopped := c.mustJSON(t, "--json", "job", "stop", host, jobID)
	if stopped.str(t, "state") != "stopped" {
		t.Fatalf("job.stop state: %s", stopped.Data)
	}
}
