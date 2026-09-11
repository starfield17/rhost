package cli

import (
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/output"
	"github.com/starfield17/rhost/internal/status"
)

func newStatusCmd() *cobra.Command {
	var timeout time.Duration
	cmd := &cobra.Command{
		Use:   "status <host>",
		Short: "One snapshot of a host's system and managed state",
		Long: `Take a single, read-only snapshot of a remote host: OS and kernel, load,
CPU, memory, disk, optional accelerator telemetry, and the sessions and jobs
rhost manages on it.

A metric the probe could not read is reported as null and named in
data.unavailable — never as a zero, and never as a failed snapshot. A host with
no GPU simply has no accelerators.

For a live view, use ` + "`rhost watch`" + `; status is one refresh of exactly the
same snapshot.`,
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			host := args[0]
			audit := startAudit("status", host)
			a := app.NewDefault()
			res, aerr := a.Status(cmd.Context(), app.StatusOptions{Host: host, Timeout: timeout})
			if aerr != nil {
				audit.fail(aerr)
				emitFailure("status", host, aerr)
				return nil
			}
			audit.succeed("", "", nil)
			if jsonFlag {
				_ = output.Success("status", host, res).Write(os.Stdout)
				return nil
			}
			renderStatus(os.Stdout, host, res)
			return nil
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "probe timeout")
	return cmd
}

// renderStatus prints the human view. Every fact here also appears in the JSON,
// so human and agent see the same snapshot (§29, AGENTS.md §6).
func renderStatus(w io.Writer, host string, r app.StatusResult) {
	fmt.Fprintln(w, host)
	if !r.Online {
		output.Field(w, "Connection", "offline · "+r.OfflineCode)
		if r.OfflineMessage != "" {
			output.Field(w, "Reason", r.OfflineMessage)
		}
		return
	}

	output.Field(w, "Connection", fmt.Sprintf("online · probe %d ms", r.ProbeMS))
	output.Field(w, "OS", osLine(r))
	output.Field(w, "Kernel", orUnknown(r.System.Kernel))
	output.Field(w, "Uptime", humanUptime(r.System.UptimeSeconds))
	output.Field(w, "CPU", cpuLine(r))
	output.Field(w, "Memory", memLine(r))

	if len(r.System.Disks) == 0 {
		output.Field(w, "Disk", "unavailable")
	}
	for _, d := range r.System.Disks {
		output.Field(w, "Disk "+d.Mount, diskLine(d))
	}
	output.Field(w, "Load", loadLine(r))

	if len(r.Accelerators) > 0 {
		fmt.Fprintln(w)
		fmt.Fprintln(w, "Accelerators")
		for _, a := range r.Accelerators {
			fmt.Fprintf(w, "  %s\n", accelLine(a))
		}
	}

	fmt.Fprintln(w)
	fmt.Fprintln(w, "Sessions")
	if len(r.Sessions) == 0 {
		fmt.Fprintln(w, "  (none)")
	}
	for _, s := range r.Sessions {
		fmt.Fprintf(w, "  %-14s %-7s %s\n", s.Name, s.Status, orDash(s.InitialCwd))
	}

	fmt.Fprintln(w)
	fmt.Fprintln(w, "Jobs")
	if len(r.Jobs) == 0 {
		fmt.Fprintln(w, "  (none)")
	}
	for _, j := range r.Jobs {
		fmt.Fprintf(w, "  %-14s %-9s %-8s %s\n", jobLabel(j), j.State, humanAge(j.StartedAt), trimOneLine(j.Command))
	}

	if len(r.Unavailable) > 0 {
		fmt.Fprintln(w)
		output.Field(w, "Unavailable", strings.Join(r.Unavailable, ", "))
	}
}

func osLine(r app.StatusResult) string {
	os := orUnknown(r.System.OS)
	detail := strings.Trim(strings.Join([]string{r.System.Platform, r.System.Arch}, ", "), ", ")
	if detail == "" {
		return os
	}
	return os + " (" + detail + ")"
}

func cpuLine(r app.StatusResult) string {
	parts := []string{}
	if r.System.CPUPercent != nil {
		parts = append(parts, fmt.Sprintf("%.1f%%", *r.System.CPUPercent))
	} else {
		parts = append(parts, "utilization unavailable")
	}
	if r.System.CPUCount != nil {
		parts = append(parts, fmt.Sprintf("%d cores", *r.System.CPUCount))
	}
	return strings.Join(parts, "   ")
}

func memLine(r app.StatusResult) string {
	m := r.System.Memory
	if m.TotalBytes == nil {
		return "unavailable"
	}
	total := humanBytes(int64(*m.TotalBytes))
	if m.UsedBytes == nil {
		return "? / " + total
	}
	return humanBytes(int64(*m.UsedBytes)) + " / " + total
}

func loadLine(r app.StatusResult) string {
	f := func(p *float64) string {
		if p == nil {
			return "?"
		}
		return fmt.Sprintf("%.2f", *p)
	}
	return fmt.Sprintf("%s  %s  %s", f(r.System.Load.One), f(r.System.Load.Five), f(r.System.Load.Fifteen))
}

func diskLine(d status.Disk) string {
	if d.TotalBytes == nil {
		return "unavailable"
	}
	total := humanBytes(int64(*d.TotalBytes))
	used := "?"
	if d.UsedBytes != nil {
		used = humanBytes(int64(*d.UsedBytes))
	}
	pct := ""
	if d.UsedPercent != nil {
		pct = fmt.Sprintf(" (%.0f%%)", *d.UsedPercent)
	}
	return used + " / " + total + pct
}

func accelLine(a status.Accelerator) string {
	parts := []string{a.Name}
	if a.UtilizationPercent != nil {
		parts = append(parts, fmt.Sprintf("util %.0f%%", *a.UtilizationPercent))
	}
	if a.MemoryUsedBytes != nil && a.MemoryTotalBytes != nil {
		parts = append(parts, fmt.Sprintf("vram %s/%s",
			humanBytes(int64(*a.MemoryUsedBytes)), humanBytes(int64(*a.MemoryTotalBytes))))
	}
	if a.TemperatureC != nil {
		parts = append(parts, fmt.Sprintf("%.0f°C", *a.TemperatureC))
	}
	return strings.Join(parts, "   ")
}

func jobLabel(j app.JobInfo) string {
	if j.Name != "" && j.Name != j.ID {
		return j.ID + " (" + j.Name + ")"
	}
	return j.ID
}

// humanUptime renders seconds as a coarse "3d 11h" style age.
func humanUptime(sec *int64) string {
	if sec == nil || *sec < 0 {
		return "unavailable"
	}
	return humanAgeSeconds(*sec)
}

// humanAge renders an RFC3339 timestamp as an age relative to now. An unparseable
// or empty timestamp renders as a dash rather than a wrong number.
func humanAge(rfc3339 string) string {
	if rfc3339 == "" {
		return "-"
	}
	t, err := time.Parse(time.RFC3339, rfc3339)
	if err != nil {
		return "-"
	}
	d := time.Since(t)
	if d < 0 {
		d = 0
	}
	return humanAgeSeconds(int64(d.Seconds()))
}

func humanAgeSeconds(sec int64) string {
	switch {
	case sec >= 86400:
		return fmt.Sprintf("%dd %dh", sec/86400, (sec%86400)/3600)
	case sec >= 3600:
		return fmt.Sprintf("%dh %dm", sec/3600, (sec%3600)/60)
	case sec >= 60:
		return fmt.Sprintf("%dm %ds", sec/60, sec%60)
	default:
		return fmt.Sprintf("%ds", sec)
	}
}

func orUnknown(s string) string {
	if strings.TrimSpace(s) == "" {
		return "unknown"
	}
	return s
}

func orDash(s string) string {
	if strings.TrimSpace(s) == "" {
		return "-"
	}
	return s
}

func trimOneLine(s string) string {
	s = strings.ReplaceAll(s, "\n", " ")
	if len(s) > 60 {
		return s[:60] + "…"
	}
	return s
}
