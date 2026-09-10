// Package status models a remote host's system telemetry for `rhost status` and
// `rhost watch` (docs/ARCHITECTURE.md §29–§31).
//
// The remote side is a single read-only probe (see ProbeScript) that prints a
// versioned `key=value` intermediate form; this package parses it into the
// structured snapshot rhost exposes. One probe process is deliberate: §30
// forbids "one SSH process per metric".
//
// Every numeric field is a pointer on purpose. A metric the probe could not read
// is nil — JSON null — and never a zero that reads like a real measurement
// (§29: an unsupported field is "unavailable", not a failure and not a 0).
package status

// ProbeVersion is the version of the intermediate form ProbeScript emits and
// ParseProbe understands. It changes only when the line format changes, so a
// parser expectation is explicit rather than implied (§30).
const ProbeVersion = 1

// Load is the 1/5/15-minute load average.
type Load struct {
	One     *float64 `json:"1"`
	Five    *float64 `json:"5"`
	Fifteen *float64 `json:"15"`
}

// Memory is the host's physical memory, in bytes.
type Memory struct {
	TotalBytes     *uint64 `json:"total_bytes"`
	AvailableBytes *uint64 `json:"available_bytes"`
	UsedBytes      *uint64 `json:"used_bytes"`
}

// Disk is one filesystem mount relevant to the user (the root and home).
type Disk struct {
	Mount          string   `json:"mount"`
	Filesystem     string   `json:"filesystem,omitempty"`
	TotalBytes     *uint64  `json:"total_bytes"`
	UsedBytes      *uint64  `json:"used_bytes"`
	AvailableBytes *uint64  `json:"available_bytes"`
	UsedPercent    *float64 `json:"used_percent"`
}

// Accelerator is one compute accelerator (today: one NVIDIA GPU per row of
// `nvidia-smi`). Fields a driver does not report are nil rather than fabricated.
type Accelerator struct {
	Vendor             string   `json:"vendor"`
	Type               string   `json:"type"`
	Name               string   `json:"name"`
	UtilizationPercent *float64 `json:"utilization_percent"`
	MemoryUsedBytes    *uint64  `json:"memory_used_bytes"`
	MemoryTotalBytes   *uint64  `json:"memory_total_bytes"`
	TemperatureC       *float64 `json:"temperature_c"`
}

// System is the generic, host-oriented telemetry view. It knows nothing about
// any particular project (§29).
type System struct {
	Hostname      string   `json:"hostname,omitempty"`
	OS            string   `json:"os,omitempty"`
	Kernel        string   `json:"kernel,omitempty"`
	Arch          string   `json:"arch,omitempty"`
	Platform      string   `json:"platform,omitempty"` // linux | wsl
	UptimeSeconds *int64   `json:"uptime_seconds"`
	Load          Load     `json:"load"`
	CPUCount      *int     `json:"cpu_count"`
	CPUPercent    *float64 `json:"cpu_percent"`
	Memory        Memory   `json:"memory"`
	Disks         []Disk   `json:"disks"`
}

// Snapshot is everything one probe run produced.
type Snapshot struct {
	System       System
	Accelerators []Accelerator
	// Unavailable names the metrics the probe did not return, so a consumer can
	// tell "this host has no GPU" from "this field could not be read" (§29).
	Unavailable []string
}
