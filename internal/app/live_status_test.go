package app

import (
	"encoding/json"
	"strings"
	"testing"
)

// TestLiveStatus is docs/ARCHITECTURE.md §48 M5 against a real host: one snapshot
// that carries the system model, an accelerator list that may legitimately be
// empty, and rhost's own sessions and jobs — none of which may be null.
func TestLiveStatus(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	env := c.mustJSON(t, "--json", "status", host)
	if !env.bool(t, "online") {
		t.Fatal("status reported online=false for a reachable host")
	}

	var sys struct {
		Hostname      string   `json:"hostname"`
		OS            string   `json:"os"`
		Kernel        string   `json:"kernel"`
		Arch          string   `json:"arch"`
		Platform      string   `json:"platform"`
		UptimeSeconds *int64   `json:"uptime_seconds"`
		CPUCount      *int     `json:"cpu_count"`
		CPUPercent    *float64 `json:"cpu_percent"`
		Memory        struct {
			TotalBytes *uint64 `json:"total_bytes"`
			UsedBytes  *uint64 `json:"used_bytes"`
		} `json:"memory"`
		Disks []struct {
			Mount string `json:"mount"`
		} `json:"disks"`
	}
	env.field(t, "system", &sys)

	if sys.Hostname == "" {
		t.Error("system.hostname is empty")
	}
	if sys.OS == "" {
		t.Error("system.os is empty")
	}
	if sys.CPUCount == nil || *sys.CPUCount < 1 {
		t.Errorf("system.cpu_count = %v, want >= 1", sys.CPUCount)
	}
	if sys.Memory.TotalBytes == nil || *sys.Memory.TotalBytes == 0 {
		t.Errorf("system.memory.total_bytes = %v, want > 0", sys.Memory.TotalBytes)
	}
	if len(sys.Disks) == 0 {
		t.Error("system.disks is empty: the root filesystem must be reported")
	}

	// Sections must be JSON arrays even when empty, so an agent can iterate.
	for _, key := range []string{"accelerators", "sessions", "jobs"} {
		var v []json.RawMessage
		env.field(t, key, &v)
	}

	// The human view must render the same snapshot.
	stdout, _, _ := c.run(t, "status", host)
	for _, want := range []string{"Connection", "Sessions", "Jobs", "online"} {
		if !strings.Contains(stdout, want) {
			t.Errorf("human status is missing %q:\n%s", want, stdout)
		}
	}
}

// TestLiveWatchStreams pins the streaming contract: one complete envelope per
// refresh, one per line, and a bounded run exits cleanly.
func TestLiveWatchStreams(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	stdout, stderr, code := c.run(t, "--json", "watch", host, "--count", "2", "--interval", "1s")
	if code != 0 {
		t.Fatalf("watch exit = %d, want 0\nstderr=%s", code, stderr)
	}
	lines := nonEmptyLines(stdout)
	if len(lines) != 2 {
		t.Fatalf("want 2 NDJSON documents, got %d:\n%s", len(lines), stdout)
	}
	for i, line := range lines {
		var env envelope
		if err := json.Unmarshal([]byte(line), &env); err != nil {
			t.Fatalf("line %d is not one JSON document: %v\n%q", i, err, line)
		}
		if !env.OK {
			t.Errorf("line %d: ok=false", i)
		}
		if !env.bool(t, "online") {
			t.Errorf("line %d: online=false against a reachable host", i)
		}
	}
}

// TestLiveWatchOffline is §48's disconnect behaviour, made deterministic with a
// name that cannot resolve: watch must stay alive, report the host offline with a
// machine-readable code, and exit cleanly on --count. It owns no state to get
// stuck on.
func TestLiveWatchOffline(t *testing.T) {
	liveHost(t) // gate on RHOST_TEST_LIVE=1; this test needs ssh, not the target host
	c := cli(t)
	const unreachable = "rhost-no-such-host.invalid"

	stdout, stderr, code := c.run(t, "--json", "watch", unreachable,
		"--count", "2", "--interval", "1s", "--timeout", "8s")
	if code != 0 {
		t.Fatalf("watch exit = %d, want 0 (offline is not a fatal error)\nstderr=%s", code, stderr)
	}
	lines := nonEmptyLines(stdout)
	if len(lines) != 2 {
		t.Fatalf("want 2 NDJSON documents, got %d:\n%s", len(lines), stdout)
	}
	env := decodeLine(t, lines[0])
	if !env.OK {
		t.Error("an offline refresh must still be a successful envelope (the monitor worked)")
	}
	if env.bool(t, "online") {
		t.Error("online=true for a host that cannot resolve")
	}
	if env.str(t, "offline_code") == "" {
		t.Error("offline_code must carry the taxonomy code")
	}
}

// TestLiveStatusMatchesDoctor cross-checks the status probe against the one
// doctor already runs: both must see the same OS and kernel.
func TestLiveStatusMatchesDoctor(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	doc := c.mustJSON(t, "--json", "doctor", host)
	var sys struct {
		OS     string `json:"os"`
		Kernel string `json:"kernel"`
	}
	c.mustJSON(t, "--json", "status", host).field(t, "system", &sys)

	if got := doc.str(t, "os"); sys.OS != got {
		t.Errorf("status os = %q, doctor os = %q", sys.OS, got)
	}
	if got := doc.str(t, "kernel"); sys.Kernel != got {
		t.Errorf("status kernel = %q, doctor kernel = %q", sys.Kernel, got)
	}
}

func nonEmptyLines(s string) []string {
	var out []string
	for _, line := range strings.Split(s, "\n") {
		if strings.TrimSpace(line) != "" {
			out = append(out, line)
		}
	}
	return out
}

func decodeLine(t *testing.T, line string) envelope {
	t.Helper()
	var env envelope
	if err := json.Unmarshal([]byte(line), &env); err != nil {
		t.Fatalf("not one JSON document: %v\n%q", err, line)
	}
	return env
}
