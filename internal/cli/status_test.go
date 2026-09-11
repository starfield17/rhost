package cli

import (
	"bytes"
	"strings"
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/status"
)

func ptrVal[T any](v T) *T { return &v }

// renderStatus is the whole human view of a snapshot. Pinning it here keeps the
// claim that "every fact the human sees is also in the JSON" (§29, AGENTS.md §6)
// honest without needing a host.

func TestRenderStatusHuman(t *testing.T) {
	r := app.StatusResult{
		Online: true, ProbeMS: 12, ProbedAt: "2026-06-01T00:00:00Z",
		System: status.System{
			Hostname: "dev-box", OS: "Ubuntu 22.04", Kernel: "5.15.0",
			Arch: "x86_64", Platform: "linux",
			UptimeSeconds: ptrVal(int64(90061)), // 1d 1h 1m
			CPUCount:      ptrVal(8), CPUPercent: ptrVal(18.3),
			Memory: status.Memory{TotalBytes: ptrVal(uint64(32 << 30)), UsedBytes: ptrVal(uint64(12 << 30))},
			Disks: []status.Disk{{
				Mount: "/", TotalBytes: ptrVal(uint64(100 << 30)),
				UsedBytes: ptrVal(uint64(40 << 30)), UsedPercent: ptrVal(40.0),
			}},
		},
		Accelerators: []status.Accelerator{{
			Vendor: "nvidia", Type: "gpu", Name: "Generic Compute GPU",
			UtilizationPercent: ptrVal(93.0),
			MemoryUsedBytes:    ptrVal(uint64(6 << 30)), MemoryTotalBytes: ptrVal(uint64(8 << 30)),
			TemperatureC: ptrVal(66.0),
		}},
		Sessions: []app.SessionInfo{{ID: "s_1", Name: "dbg", Status: "alive", InitialCwd: "/home/dev/work"}},
		Jobs: []app.JobInfo{{
			ID: "j_1", Name: "train", State: "running", Command: "python train.py",
			StartedAt: time.Now().Add(-2 * time.Hour).UTC().Format(time.RFC3339),
		}},
		Unavailable: []string{"load"},
	}
	var buf bytes.Buffer
	renderStatus(&buf, "gpu", r)
	out := buf.String()

	for _, want := range []string{
		"gpu",
		"online · probe 12 ms",
		"Ubuntu 22.04 (linux, x86_64)",
		"1d 1h",
		"18.3%   8 cores",
		"40.0", // disk line shows a size, proving the disk rendered
		"Disk /",
		"(40%)",
		"Generic Compute GPU",
		"util 93%",
		"66°C",
		"dbg",
		"/home/dev/work",
		"j_1 (train)",
		"running",
		"Unavailable",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("human status is missing %q:\n%s", want, out)
		}
	}
}

func TestRenderStatusOffline(t *testing.T) {
	var buf bytes.Buffer
	renderStatus(&buf, "gpu", app.StatusResult{
		Online: false, OfflineCode: "SSH_UNREACHABLE", OfflineMessage: "host did not accept a connection",
	})
	out := buf.String()
	if !strings.Contains(out, "offline") || !strings.Contains(out, "SSH_UNREACHABLE") {
		t.Errorf("offline view must name the reason:\n%s", out)
	}
	if strings.Contains(out, "Sessions") {
		t.Errorf("offline view must not print an empty detail section:\n%s", out)
	}
}

// Unavailable metrics must render as words, never as a zero that reads like a
// measurement (§29).
func TestRenderStatusUnavailable(t *testing.T) {
	var buf bytes.Buffer
	renderStatus(&buf, "gpu", app.StatusResult{
		Online: true,
		System: status.System{UptimeSeconds: nil, CPUCount: nil, CPUPercent: nil},
	})
	out := buf.String()
	if !strings.Contains(out, "utilization unavailable") {
		t.Errorf("missing CPU utilization must say so:\n%s", out)
	}
	if !strings.Contains(out, "unavailable") {
		t.Errorf("missing uptime/disks must say so:\n%s", out)
	}
}

func TestHumanAgeSeconds(t *testing.T) {
	cases := map[int64]string{
		0:      "0s",
		59:     "59s",
		60:     "1m 0s",
		3661:   "1h 1m",
		90061:  "1d 1h",
		259200: "3d 0h",
	}
	for sec, want := range cases {
		if got := humanAgeSeconds(sec); got != want {
			t.Errorf("humanAgeSeconds(%d) = %q, want %q", sec, got, want)
		}
	}
}

func TestHumanAgeRejectsGarbage(t *testing.T) {
	if got := humanAge(""); got != "-" {
		t.Errorf("empty timestamp = %q, want -", got)
	}
	if got := humanAge("not-a-time"); got != "-" {
		t.Errorf("unparseable timestamp = %q, want -", got)
	}
}

func TestOfflineWatchTickHasTimestamp(t *testing.T) {
	res := offlineStatus(errs.New(errs.SSHUnreachable, "offline", true))
	if _, err := time.Parse(time.RFC3339, res.ProbedAt); err != nil {
		t.Errorf("offline watch timestamp = %q: %v", res.ProbedAt, err)
	}
}
