package fileops

import (
	"os"
	"os/exec"
	"testing"
)

// TestEmbeddedRemoteHelperSuite runs remote.py's own suite, from Go, so the
// helper that answers `fs read/write/patch` on the far side is covered
// by `make check` rather than by whoever remembered to run python by hand.
//
// It exercises the helper exactly as the remote does — one JSON request in, one
// JSON response out — so a change to the response shape has to be made twice:
// here and in the CLI. That is the point: the shapes asserted there are the
// shapes an agent reads under `--json` (AGENTS.md §6).
//
// Set RHOST_PYTHON to name a specific interpreter. Python 3 is mandatory; the
// live SSH suite (internal/app) still covers the real remote path.
func TestEmbeddedRemoteHelperSuite(t *testing.T) {
	python := os.Getenv("RHOST_PYTHON")
	if python == "" {
		python = "python3"
	}
	path, err := exec.LookPath(python)
	if err != nil {
		t.Fatalf("no %s on PATH: remote_test.py (the embedded helper's own suite) is mandatory", python)
	}
	cmd := exec.Command(path, "-m", "unittest", "discover", "-s", ".", "-p", "remote_test.py")
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Errorf("remote.py suite failed: %v\n%s", err, out)
	}
}
