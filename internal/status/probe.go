package status

import (
	"fmt"
	"strconv"
	"strings"
)

// ProbeScript is the single, read-only remote probe behind `status` and `watch`.
//
// It runs through the normal exec wrapper (bash -lc), so it must be a POSIX-ish
// bash script that always exits 0 and prints the intermediate form documented by
// ParseProbe. It reads only /proc, uname, df and the optional nvidia-smi; it
// writes nothing and installs nothing (§30).
//
// The one real cost is the fixed ~0.3s delay used to measure CPU utilization: a
// one-shot agent cannot observe a rate without two samples, and paying it once
// here is cheaper and more honest than reporting load average as if it were
// utilization.
const ProbeScript = `em() { printf '%s=%s\n' "$1" "$2"; }
printf 'rhost_probe_version=1\n'

em hostname "$(uname -n 2>/dev/null)"
em kernel "$(uname -r 2>/dev/null)"
em arch "$(uname -m 2>/dev/null)"

os=""
if [ -r /etc/os-release ]; then
  . /etc/os-release 2>/dev/null
  os="${PRETTY_NAME:-${NAME:-}}"
fi
[ -n "$os" ] || os="$(uname -s 2>/dev/null)"
em os "$os"

if grep -qiE 'microsoft|wsl' /proc/version 2>/dev/null; then em platform wsl; else em platform linux; fi

if [ -r /proc/uptime ]; then
  read -r up _ < /proc/uptime
  em uptime_seconds "${up%.*}"
fi

if [ -r /proc/loadavg ]; then
  read -r l1 l5 l15 _ < /proc/loadavg
  em load1 "$l1"; em load5 "$l5"; em load15 "$l15"
fi

cc="$(getconf _NPROCESSORS_ONLN 2>/dev/null)"
[ -n "$cc" ] || cc="$(nproc 2>/dev/null)"
[ -n "$cc" ] && em cpu_count "$cc"

if [ -r /proc/stat ]; then
  rhost_cpu_snap() {
    set -- $(head -n1 /proc/stat 2>/dev/null)
    shift
    _t=0; for _x in "$@"; do _t=$((_t + _x)); done
    printf '%s %s' "$_t" "$(( ${4:-0} + ${5:-0} ))"
  }
  s1=$(rhost_cpu_snap)
  sleep 0.3
  s2=$(rhost_cpu_snap)
  if [ -n "$s1" ] && [ -n "$s2" ]; then
    t1=${s1%% *}; i1=${s1##* }; t2=${s2%% *}; i2=${s2##* }
    dt=$((t2 - t1)); di=$((i2 - i1))
    if [ "$dt" -gt 0 ]; then
      p=$(( (dt - di) * 1000 / dt ))
      em cpu_percent "$((p / 10)).$((p % 10))"
    fi
  fi
fi

if [ -r /proc/meminfo ]; then
  mt=$(awk '$1=="MemTotal:"{print $2; exit}' /proc/meminfo)
  ma=$(awk '$1=="MemAvailable:"{print $2; exit}' /proc/meminfo)
  if [ -z "$ma" ]; then
    mf=$(awk '$1=="MemFree:"{print $2; exit}' /proc/meminfo)
    mb=$(awk '$1=="Buffers:"{print $2; exit}' /proc/meminfo)
    mc=$(awk '$1=="Cached:"{print $2; exit}' /proc/meminfo)
    [ -n "$mf" ] && ma=$((mf + ${mb:-0} + ${mc:-0}))
  fi
  [ -n "$mt" ] && em mem_total_kb "$mt"
  [ -n "$ma" ] && em mem_available_kb "$ma"
fi

# The root and the home, which are the filesystems a user's work lands on.
for _m in / "$HOME"; do
  [ -n "$_m" ] || continue
  df -Pk "$_m" 2>/dev/null | awk 'NR==2 && $2 ~ /^[0-9]+$/ { printf "disk=%s\t%s\t%s\t%s\t%s\t%s\n", $1, $2, $3, $4, $5, $6 }'
done

# Optional NVIDIA telemetry. Missing nvidia-smi is normal (WSL, CPU-only, macOS
# CI) and must not fail the snapshot (§29).
if command -v nvidia-smi >/dev/null 2>&1; then
  nvidia-smi --query-gpu=index,name,utilization.gpu,memory.used,memory.total,temperature.gpu \
    --format=csv,noheader,nounits 2>/dev/null | awk -F',' '{
      for (i = 1; i <= NF; i++) { gsub(/^[ \t]+|[ \t]+$/, "", $i) }
      printf "gpu=%s\t%s\t%s\t%s\t%s\t%s\n", $1, $2, $3, $4, $5, $6
    }'
fi
:`

// ParseProbe turns the intermediate form into a Snapshot.
//
// It is strict about the one thing that makes the rest meaningful — the version
// line — and tolerant about everything else: an unknown key is ignored, a
// malformed value leaves that metric nil, and a missing metric is recorded in
// Snapshot.Unavailable. That way a host that lacks /proc/meminfo still produces a
// usable snapshot rather than an error.
func ParseProbe(out string) (Snapshot, error) {
	var (
		snap     Snapshot
		version  = -1
		have     = map[string]bool{}
		seenDisk = map[string]bool{}
	)
	snap.System.Disks = []Disk{}
	snap.Accelerators = []Accelerator{}

	for _, raw := range strings.Split(out, "\n") {
		line := strings.TrimRight(raw, "\r")
		if line == "" {
			continue
		}
		key, val, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}
		have[key] = true

		switch key {
		case "rhost_probe_version":
			if n, err := strconv.Atoi(strings.TrimSpace(val)); err == nil {
				version = n
			}
		case "hostname":
			snap.System.Hostname = val
		case "os":
			snap.System.OS = val
		case "kernel":
			snap.System.Kernel = val
		case "arch":
			snap.System.Arch = val
		case "platform":
			snap.System.Platform = val
		case "uptime_seconds":
			snap.System.UptimeSeconds = parseInt64(val)
		case "load1":
			snap.System.Load.One = parseFloat(val)
		case "load5":
			snap.System.Load.Five = parseFloat(val)
		case "load15":
			snap.System.Load.Fifteen = parseFloat(val)
		case "cpu_count":
			snap.System.CPUCount = parseInt(val)
		case "cpu_percent":
			snap.System.CPUPercent = parseFloat(val)
		case "mem_total_kb":
			snap.System.Memory.TotalBytes = kbToBytes(parseUint64(val))
		case "mem_available_kb":
			snap.System.Memory.AvailableBytes = kbToBytes(parseUint64(val))
		case "disk":
			if d, mount, ok := parseDisk(val); ok && !seenDisk[mount] {
				seenDisk[mount] = true
				snap.System.Disks = append(snap.System.Disks, d)
			}
		case "gpu":
			if a, ok := parseGPU(val); ok {
				snap.Accelerators = append(snap.Accelerators, a)
			}
		}
	}

	if version != ProbeVersion {
		return Snapshot{}, fmt.Errorf("status probe version %d, want %d", version, ProbeVersion)
	}

	snap.System.Memory.UsedBytes = subBytes(snap.System.Memory.TotalBytes, snap.System.Memory.AvailableBytes)
	snap.Unavailable = missing(have, snap)
	return snap, nil
}

// parseDisk reads `filesystem<TAB>total_kb<TAB>used_kb<TAB>avail_kb<TAB>pct<TAB>mount`.
func parseDisk(val string) (Disk, string, bool) {
	f := strings.Split(val, "\t")
	if len(f) < 6 {
		return Disk{}, "", false
	}
	pct := parseFloat(strings.TrimSuffix(strings.TrimSpace(f[4]), "%"))
	return Disk{
		Filesystem:     strings.TrimSpace(f[0]),
		TotalBytes:     kbToBytes(parseUint64(f[1])),
		UsedBytes:      kbToBytes(parseUint64(f[2])),
		AvailableBytes: kbToBytes(parseUint64(f[3])),
		UsedPercent:    pct,
		Mount:          strings.TrimSpace(f[5]),
	}, strings.TrimSpace(f[5]), true
}

// parseGPU reads `index<TAB>name<TAB>util<TAB>mem_used_mib<TAB>mem_total_mib<TAB>temp`.
func parseGPU(val string) (Accelerator, bool) {
	f := strings.Split(val, "\t")
	if len(f) < 6 || strings.TrimSpace(f[1]) == "" {
		return Accelerator{}, false
	}
	return Accelerator{
		Vendor:             "nvidia",
		Type:               "gpu",
		Name:               strings.TrimSpace(f[1]),
		UtilizationPercent: parseFloat(f[2]),
		MemoryUsedBytes:    mibToBytes(parseUint64(f[3])),
		MemoryTotalBytes:   mibToBytes(parseUint64(f[4])),
		TemperatureC:       parseFloat(f[5]),
	}, true
}

// missing names the metrics a snapshot could not obtain. Accelerators are not in
// the list: a host with no GPU legitimately has none.
func missing(have map[string]bool, snap Snapshot) []string {
	var u []string
	if !have["uptime_seconds"] {
		u = append(u, "uptime")
	}
	if !have["load1"] {
		u = append(u, "load")
	}
	if !have["cpu_count"] {
		u = append(u, "cpu_count")
	}
	if !have["cpu_percent"] {
		u = append(u, "cpu_percent")
	}
	if !have["mem_total_kb"] {
		u = append(u, "memory")
	}
	if len(snap.System.Disks) == 0 {
		u = append(u, "disk")
	}
	return u
}

// unavailableTokens are the placeholders nvidia-smi (and similar tools) print
// for a field the driver cannot report. They must become nil, not a parse of 0.
func isUnavailable(s string) bool {
	t := strings.TrimSpace(s)
	return t == "" || t == "[N/A]" || t == "[Not Supported]" || strings.EqualFold(t, "n/a")
}

func parseFloat(s string) *float64 {
	if isUnavailable(s) {
		return nil
	}
	v, err := strconv.ParseFloat(strings.TrimSpace(s), 64)
	if err != nil {
		return nil
	}
	return &v
}

func parseInt(s string) *int {
	if isUnavailable(s) {
		return nil
	}
	v, err := strconv.Atoi(strings.TrimSpace(s))
	if err != nil {
		return nil
	}
	return &v
}

func parseInt64(s string) *int64 {
	if isUnavailable(s) {
		return nil
	}
	v, err := strconv.ParseInt(strings.TrimSpace(s), 10, 64)
	if err != nil {
		return nil
	}
	return &v
}

func parseUint64(s string) *uint64 {
	if isUnavailable(s) {
		return nil
	}
	v, err := strconv.ParseUint(strings.TrimSpace(s), 10, 64)
	if err != nil {
		return nil
	}
	return &v
}

func kbToBytes(kb *uint64) *uint64 {
	if kb == nil {
		return nil
	}
	b := *kb * 1024
	return &b
}

func mibToBytes(mib *uint64) *uint64 {
	if mib == nil {
		return nil
	}
	b := *mib * 1024 * 1024
	return &b
}

// subBytes returns a-b in bytes, or nil if either operand is unknown.
func subBytes(a, b *uint64) *uint64 {
	if a == nil || b == nil || *b > *a {
		return nil
	}
	d := *a - *b
	return &d
}
