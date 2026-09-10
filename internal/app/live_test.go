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
