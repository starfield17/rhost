package cli

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/audit"
	"github.com/starfield17/rhost/internal/config"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

// auditTimer records one operation when it finishes (docs/ARCHITECTURE.md §36).
//
// Auditing is fail-open: a write error is reported on stderr and the operation
// is unaffected, because a full or read-only disk must not make remote work
// impossible. When auditing is disabled (RHOST_AUDIT=0) the timer is inert, so a
// call site never branches on whether it is on.
type auditTimer struct {
	start time.Time
	rec   *audit.Recorder
	op    string
	host  string
}

func startAudit(op, host string) *auditTimer {
	t := &auditTimer{start: time.Now(), op: op, host: host}
	if audit.Enabled() {
		t.rec = &audit.Recorder{Path: audit.Path(config.StateDir())}
	}
	return t
}

// succeed records a completed operation. cwd and command may be empty; exitCode
// is nil for operations that have no remote status of their own.
func (t *auditTimer) succeed(cwd, command string, exitCode *int) {
	t.write(audit.Entry{OK: true, Cwd: cwd, Command: audit.Summarize(command), ExitCode: exitCode})
}

// fail records a refused or failed operation.
func (t *auditTimer) fail(aerr *errs.Error) {
	e := audit.Entry{OK: false}
	if aerr != nil {
		e.ErrorCode = string(aerr.Code)
	}
	t.write(e)
}

func (t *auditTimer) write(e audit.Entry) {
	if t == nil || t.rec == nil {
		return
	}
	e.Time = time.Now().UTC().Format(time.RFC3339)
	e.Operation = t.op
	e.Host = t.host
	e.DurationMS = time.Since(t.start).Milliseconds()
	if err := t.rec.Record(e); err != nil {
		fmt.Fprintf(os.Stderr, "rhost: audit: %v\n", err)
	}
}

// auditListing is the JSON payload of `rhost audit`.
type auditListing struct {
	Path    string        `json:"path"`
	Entries []audit.Entry `json:"entries"`
}

func newAuditCmd() *cobra.Command {
	var (
		limit int
		host  string
	)
	cmd := &cobra.Command{
		Use:   "audit",
		Short: "Read the local audit log of remote operations",
		Long: `Read rhost's local audit log — a JSON Lines file of the operations this
machine has run against remote hosts (docs/ARCHITECTURE.md §36).

It records operations, not secrets: no environment maps and no file contents.
Logging is fail-open and can be turned off with RHOST_AUDIT=0.

By default the most recent 20 entries are shown; --limit 0 shows all.`,
		Args: cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			path := audit.Path(config.StateDir())
			entries, aerr := readAuditLog(path)
			if aerr != nil {
				emitFailure("audit", "", aerr)
				return nil
			}
			if host != "" {
				entries = filterAuditHost(entries, host)
			}
			if limit > 0 && len(entries) > limit {
				entries = entries[len(entries)-limit:]
			}
			if jsonFlag {
				_ = output.Success("audit", "", auditListing{Path: path, Entries: entries}).Write(os.Stdout)
				return nil
			}
			renderAudit(os.Stdout, path, entries)
			return nil
		},
	}
	cmd.Flags().IntVar(&limit, "limit", 20, "show at most the last N entries (0 = all)")
	cmd.Flags().StringVar(&host, "host", "", "only entries for this host")
	return cmd
}

// readAuditLog parses the JSONL file. A missing file is an empty log, not an
// error; a corrupt line is skipped rather than making the whole trail unreadable,
// which matters because the log is appended to by other processes concurrently.
func readAuditLog(path string) ([]audit.Entry, *errs.Error) {
	f, err := os.Open(path)
	if os.IsNotExist(err) {
		return []audit.Entry{}, nil
	}
	if err != nil {
		return nil, errs.Wrap(errs.ConfigInvalid, "cannot read audit log: "+err.Error(), false, err)
	}
	defer f.Close()

	out := []audit.Entry{}
	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" {
			continue
		}
		var e audit.Entry
		if err := json.Unmarshal([]byte(line), &e); err != nil {
			continue
		}
		out = append(out, e)
	}
	if err := sc.Err(); err != nil {
		return nil, errs.Wrap(errs.ConfigInvalid, "reading audit log: "+err.Error(), false, err)
	}
	return out, nil
}

func filterAuditHost(entries []audit.Entry, host string) []audit.Entry {
	out := []audit.Entry{}
	for _, e := range entries {
		if e.Host == host {
			out = append(out, e)
		}
	}
	return out
}

func renderAudit(w io.Writer, path string, entries []audit.Entry) {
	if len(entries) == 0 {
		fmt.Fprintf(w, "no audit entries (%s)\n", path)
		return
	}
	for _, e := range entries {
		status := "ok"
		if !e.OK {
			status = e.ErrorCode
			if status == "" {
				status = "failed"
			}
		}
		fmt.Fprintf(w, "%s  %-10s %-18s %-8s %s\n", e.Time, dash(e.Host), e.Operation, status, e.Command)
	}
}

func dash(s string) string {
	if s == "" {
		return "-"
	}
	return s
}
