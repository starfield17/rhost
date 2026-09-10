package app

import (
	"context"
	"os"
	"strings"
	"testing"
	"time"
)

// TestLiveExec exercises the real Mac -> remote-host path. It only runs when
// explicitly enabled, so ordinary unit tests never touch a real machine.
//
//	RHOST_TEST_LIVE=1 RHOST_TEST_HOST=orangepi@192.168.123.179 go test ./internal/app/
func TestLiveExec(t *testing.T) {
	if os.Getenv("RHOST_TEST_LIVE") != "1" {
		t.Skip("set RHOST_TEST_LIVE=1 to run live tests")
	}
	host := os.Getenv("RHOST_TEST_HOST")
	if host == "" {
		t.Skip("set RHOST_TEST_HOST")
	}

	a := NewDefault()
	ctx := context.Background()

	res, aerr := a.Execute(ctx, ExecOptions{
		Host:    host,
		Command: "echo live-ok; exit 4",
		Timeout: 60 * time.Second,
	})
	if aerr != nil {
		t.Fatalf("Execute returned adapter error: %s: %s", aerr.Code, aerr.Message)
	}
	if res.ExitCode != 4 {
		t.Errorf("exit code = %d, want 4", res.ExitCode)
	}
	if strings.TrimSpace(res.Stdout) != "live-ok" {
		t.Errorf("stdout = %q, want live-ok", res.Stdout)
	}

	doc, aerr := a.Doctor(ctx, host, 60*time.Second)
	if aerr != nil {
		t.Fatalf("Doctor returned adapter error: %s: %s", aerr.Code, aerr.Message)
	}
	if !doc.Online || !doc.Capabilities["bash"] {
		t.Errorf("unexpected doctor result: %+v", doc)
	}
}

// TestLiveSession proves the product-defining session behaviour: shell state
// (cwd) persists across separate calls, and output is readable incrementally.
func TestLiveSession(t *testing.T) {
	if os.Getenv("RHOST_TEST_LIVE") != "1" {
		t.Skip("set RHOST_TEST_LIVE=1 to run live tests")
	}
	host := os.Getenv("RHOST_TEST_HOST")
	if host == "" {
		t.Skip("set RHOST_TEST_HOST")
	}

	a := NewDefault()
	ctx := context.Background()
	const name = "livetest"

	if _, aerr := a.SessionCreate(ctx, host, name, "/tmp", "bash", 90*time.Second); aerr != nil {
		t.Fatalf("SessionCreate: %s: %s", aerr.Code, aerr.Message)
	}
	defer a.SessionClose(ctx, host, name, 30*time.Second)

	if _, aerr := a.SessionExec(ctx, host, name, "cd /var/log && echo moved", 60*time.Second); aerr != nil {
		t.Fatalf("SessionExec cd: %s: %s", aerr.Code, aerr.Message)
	}
	res, aerr := a.SessionExec(ctx, host, name, "pwd", 60*time.Second)
	if aerr != nil {
		t.Fatalf("SessionExec pwd: %s: %s", aerr.Code, aerr.Message)
	}
	if got := strings.TrimSpace(res.Output); got != "/var/log" {
		t.Errorf("cwd not persisted: got %q, want /var/log", got)
	}

	if _, aerr := a.SessionExec(ctx, host, name, "exit 7", 30*time.Second); aerr == nil {
		t.Errorf("expected an adapter error when the command runs `exit`")
	}
}
