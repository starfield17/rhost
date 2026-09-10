package status

import (
	"fmt"
	"strings"
	"testing"
)

// The probe parser is the piece of `status` that can be pinned without a host.
// These tests feed it the exact intermediate form ProbeScript emits — including
// the shapes that come from real hosts: a WSL box with no GPU, a driver that
// reports [N/A], a truncated /proc, and a stray line that is not key=value.

func mustParse(t *testing.T, out string) Snapshot {
	t.Helper()
	s, err := ParseProbe(out)
	if err != nil {
		t.Fatalf("ParseProbe: %v", err)
	}
	return s
}

// The probe prints its version as a literal inside the script while ProbeVersion
// is a Go constant; this pins them together so they cannot drift silently — a
// mismatch would make every real probe fail to parse.
func TestProbeScriptVersionMatchesConstant(t *testing.T) {
	line := fmt.Sprintf("rhost_probe_version=%d", ProbeVersion)
	if !strings.Contains(ProbeScript, line) {
		t.Errorf("ProbeScript must contain %q (bump the script and ProbeVersion together)", line)
	}
}

const fullProbe = `rhost_probe_version=1
hostname=dev-box
kernel=5.15.0-91-generic
arch=x86_64
os=Ubuntu 22.04.3 LTS
platform=linux
uptime_seconds=323456
load1=0.15
load5=0.10
load15=0.08
cpu_count=8
cpu_percent=18.3
mem_total_kb=32768000
mem_available_kb=11468800
disk=/dev/sda1	950000000	380000000	570000000	40%	/
disk=/dev/sdb1	1000000	500000	500000	50%	/home/dev
gpu=0	NVIDIA GeForce RTX 4060	93	6340	8188	66
`

func TestParseProbeFull(t *testing.T) {
	s := mustParse(t, fullProbe)

	if s.System.Hostname != "dev-box" || s.System.OS != "Ubuntu 22.04.3 LTS" {
		t.Errorf("identity = %+v", s.System)
	}
	if s.System.Platform != "linux" {
		t.Errorf("platform = %q", s.System.Platform)
	}
	if got := *s.System.UptimeSeconds; got != 323456 {
		t.Errorf("uptime = %d", got)
	}
	if got := *s.System.CPUCount; got != 8 {
		t.Errorf("cpu_count = %d", got)
	}
	if got := *s.System.CPUPercent; got != 18.3 {
		t.Errorf("cpu_percent = %v", got)
	}
	if got := *s.System.Load.One; got != 0.15 {
		t.Errorf("load1 = %v", got)
	}

	// Child-process units arrive as KiB/MiB and must be reported as bytes.
	if got := *s.System.Memory.TotalBytes; got != 32768000*1024 {
		t.Errorf("mem total = %d", got)
	}
	if got := *s.System.Memory.AvailableBytes; got != 11468800*1024 {
		t.Errorf("mem available = %d", got)
	}
	if got := *s.System.Memory.UsedBytes; got != (32768000-11468800)*1024 {
		t.Errorf("mem used = %d, want derived total-available", got)
	}

	if len(s.System.Disks) != 2 {
		t.Fatalf("disks = %d, want 2: %+v", len(s.System.Disks), s.System.Disks)
	}
	if d := s.System.Disks[0]; d.Mount != "/" || *d.UsedPercent != 40 || *d.TotalBytes != 950000000*1024 {
		t.Errorf("disk[0] = %+v", d)
	}

	if len(s.Accelerators) != 1 {
		t.Fatalf("accelerators = %d, want 1", len(s.Accelerators))
	}
	g := s.Accelerators[0]
	if g.Vendor != "nvidia" || g.Type != "gpu" || g.Name != "NVIDIA GeForce RTX 4060" {
		t.Errorf("gpu identity = %+v", g)
	}
	if *g.UtilizationPercent != 93 || *g.TemperatureC != 66 {
		t.Errorf("gpu util/temp = %+v", g)
	}
	if *g.MemoryUsedBytes != 6340*1024*1024 || *g.MemoryTotalBytes != 8188*1024*1024 {
		t.Errorf("gpu memory = %+v", g)
	}

	if len(s.Unavailable) != 0 {
		t.Errorf("a complete probe reports unavailable = %v", s.Unavailable)
	}
}

// A WSL/CPU-only host returns no gpu lines. That is not an error and not an
// "unavailable" metric — it is a host with no accelerators (§29).
func TestParseProbeWithoutNvidia(t *testing.T) {
	s := mustParse(t, `rhost_probe_version=1
hostname=wsl-box
os=Ubuntu
platform=wsl
uptime_seconds=10
load1=0
load5=0
load15=0
cpu_count=4
cpu_percent=0.0
mem_total_kb=100
mem_available_kb=50
disk=/dev/sdc	2000	1000	1000	50%	/
`)
	if s.System.Platform != "wsl" {
		t.Errorf("platform = %q, want wsl", s.System.Platform)
	}
	if len(s.Accelerators) != 0 {
		t.Errorf("accelerators = %v, want empty", s.Accelerators)
	}
	if len(s.Unavailable) != 0 {
		t.Errorf("unavailable = %v, want none", s.Unavailable)
	}
}

// A driver that cannot report a field prints [N/A]; it must become null, never a
// zero that reads like a real measurement.
func TestParseProbeNAFields(t *testing.T) {
	s := mustParse(t, `rhost_probe_version=1
platform=linux
gpu=0	NVIDIA GeForce RTX 4060	[N/A]	[N/A]	8188	[N/A]
`)
	g := s.Accelerators[0]
	if g.UtilizationPercent != nil || g.MemoryUsedBytes != nil || g.TemperatureC != nil {
		t.Errorf("[N/A] fields must be nil, got %+v", g)
	}
	if g.MemoryTotalBytes == nil || *g.MemoryTotalBytes != 8188*1024*1024 {
		t.Errorf("the reported field must survive: %+v", g)
	}
}

// A truncated /proc yields a partial snapshot plus a list of what is missing,
// never an error.
func TestParseProbeMissingMetrics(t *testing.T) {
	s := mustParse(t, "rhost_probe_version=1\nhostname=minimal\n")
	if s.System.Hostname != "minimal" {
		t.Errorf("hostname = %q", s.System.Hostname)
	}
	if s.System.CPUPercent != nil || s.System.UptimeSeconds != nil {
		t.Errorf("missing metrics must be nil: %+v", s.System)
	}
	want := map[string]bool{"uptime": true, "load": true, "cpu_count": true, "cpu_percent": true, "memory": true, "disk": true}
	for _, u := range s.Unavailable {
		delete(want, u)
	}
	if len(want) != 0 {
		t.Errorf("unavailable = %v, missing %v", s.Unavailable, want)
	}
}

// Two rows for the same mount (e.g. HOME on the root filesystem) must not be
// reported twice.
func TestParseProbeDedupesDisks(t *testing.T) {
	s := mustParse(t, `rhost_probe_version=1
disk=/dev/sda1	100	40	60	40%	/
disk=/dev/sda1	100	40	60	40%	/
`)
	if len(s.System.Disks) != 1 {
		t.Errorf("disks = %+v, want one deduped row", s.System.Disks)
	}
}

// Stray non-key lines (a shell banner, a warning) are ignored; they must not
// shift parsing or fail the snapshot.
func TestParseProbeIgnoresNoise(t *testing.T) {
	s := mustParse(t, `rhost_probe_version=1
this line has no equals sign
hostname=dev-box
=empty-key
notanumber=wat
gpu=0
`)
	if s.System.Hostname != "dev-box" {
		t.Errorf("hostname = %q", s.System.Hostname)
	}
	if len(s.Accelerators) != 0 {
		t.Errorf("a short gpu line must be dropped, got %v", s.Accelerators)
	}
}

// The version line is the one thing the parser is strict about: without it the
// shape of every other line is unknown.
func TestParseProbeRejectsWrongVersion(t *testing.T) {
	for _, out := range []string{
		"hostname=x\n",                        // no version line at all
		"rhost_probe_version=2\nhostname=x\n", // a future format
	} {
		if _, err := ParseProbe(out); err == nil {
			t.Errorf("ParseProbe(%q) = nil error, want a version failure", out)
		}
	}
}
