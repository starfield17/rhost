package openssh

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestTunnelCheckUncertaintyPreservesRecord(t *testing.T) {
	t.Setenv("RHOST_STATE_DIR", t.TempDir())
	t.Setenv("RHOST_CACHE_DIR", t.TempDir())
	if err := os.MkdirAll(filepath.Dir(tunnelSocket("unused")), 0700); err != nil {
		t.Fatal(err)
	}
	if err := ensureTunnelRoot(); err != nil {
		t.Fatal(err)
	}
	id := "t_" + strings.Repeat("ab", 16)
	record := filepath.Join(tunnelRoot(), id+".json")
	data, _ := json.Marshal(Tunnel{ID: id, TunnelID: id, Host: "example-host", Status: TunnelAlive})
	if err := os.WriteFile(record, data, 0600); err != nil {
		t.Fatal(err)
	}
	// An existing endpoint with an unsuccessful check is not evidence of death.
	if err := os.WriteFile(tunnelSocket(id), nil, 0600); err != nil {
		t.Fatal(err)
	}
	stub := filepath.Join(t.TempDir(), "ssh")
	if err := os.WriteFile(stub, []byte("#!/bin/sh\nexit 255\n"), 0700); err != nil {
		t.Fatal(err)
	}
	c := New(Config{SSHBin: stub})
	if err := c.CloseTunnel(context.Background(), id); err == nil {
		t.Fatal("uncertain close succeeded")
	}
	if _, err := os.Stat(record); err != nil {
		t.Fatal("record lost:", err)
	}
	if _, err := c.ListTunnels(context.Background()); err == nil {
		t.Fatal("uncertain list succeeded")
	}
	cMissing := New(Config{SSHBin: filepath.Join(t.TempDir(), "missing-ssh")})
	if err := cMissing.CloseTunnel(context.Background(), id); err == nil {
		t.Fatal("missing SSH binary was treated as an absent socket")
	}
	if _, err := os.Stat(record); err != nil {
		t.Fatal("record lost:", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := c.CloseTunnel(ctx, id); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancel = %v", err)
	}
	if _, err := os.Stat(record); err != nil {
		t.Fatal("record lost:", err)
	}
	if err := os.Remove(tunnelSocket(id)); err != nil {
		t.Fatal(err)
	}
	rows, err := c.ListTunnels(context.Background())
	if err != nil || len(rows) != 1 || rows[0].Status != TunnelStale {
		t.Fatalf("absent socket: %+v, %v", rows, err)
	}
	if err := c.CloseTunnel(context.Background(), id); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(record); !os.IsNotExist(err) {
		t.Fatalf("record remains: %v", err)
	}
}
