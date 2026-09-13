// Package audit writes a local, best-effort record of rhost's remote operations
// (docs/architecture/engineering.md).
//
// It records bounded operation metadata: never an environment map, never file
// contents. Command summaries are not a secret filter. The log is a JSON Lines file — one Entry per line — so it can be
// tailed and parsed without a schema migration.
//
// Logging is deliberately fail-open: the caller is expected to report a write
// failure and carry on, because a full or read-only disk must not make remote
// work impossible (docs/architecture/engineering.md#security-and-audit).
// It can be turned off entirely with RHOST_AUDIT=0.
package audit

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"
)

// EnvVar turns auditing on or off. Any of 0/false/no/off (case-insensitive)
// disables it; anything else (including unset) leaves it on.
const EnvVar = "RHOST_AUDIT"

// maxCommand bounds the recorded command summary. The log answers "what ran",
// not "reproduce this argv", so a long command is summarised rather than stored
// whole.
const maxCommand = 200

// Entry is one audited operation. Its JSON field names are a stable surface.
type Entry struct {
	Time       string `json:"time"`
	Host       string `json:"host,omitempty"`
	Operation  string `json:"operation"`
	Cwd        string `json:"cwd,omitempty"`
	Command    string `json:"command_summary,omitempty"`
	ExitCode   *int   `json:"exit_code,omitempty"`
	DurationMS int64  `json:"duration_ms"`
	OK         bool   `json:"ok"`
	ErrorCode  string `json:"error_code,omitempty"`
}

// Enabled reports whether auditing is on, honouring EnvVar.
func Enabled() bool {
	switch strings.ToLower(strings.TrimSpace(os.Getenv(EnvVar))) {
	case "0", "false", "no", "off":
		return false
	}
	return true
}

// Path is the audit log under a state directory.
func Path(stateDir string) string { return filepath.Join(stateDir, "audit.jsonl") }

// Recorder appends Entries to one JSONL file. A nil recorder, or one with an
// empty Path, discards entries, so a caller never has to branch on whether
// auditing is enabled.
type Recorder struct {
	Path string
}

// Record appends e as a single JSON line. It creates the parent directory
// user-private (0700) and the file 0600, and the write is O_APPEND so concurrent
// rhost processes interleave whole lines rather than corrupting each other.
func (r *Recorder) Record(e Entry) error {
	if r == nil || r.Path == "" {
		return nil
	}
	line, err := json.Marshal(e)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(r.Path), 0o700); err != nil {
		return err
	}
	if err := os.Chmod(filepath.Dir(r.Path), 0o700); err != nil {
		return err
	}
	if info, err := os.Lstat(r.Path); err == nil {
		if info.Mode()&os.ModeSymlink != 0 || !info.Mode().IsRegular() {
			return &os.PathError{Op: "open", Path: r.Path, Err: os.ErrPermission}
		}
		if err := os.Chmod(r.Path, 0o600); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	f, err := os.OpenFile(r.Path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer f.Close()
	_, err = f.Write(append(line, '\n'))
	return err
}

// Summarize collapses a command to one bounded line for the log. Newlines become
// spaces (an entry is one line) and the result is truncated at a rune boundary.
func Summarize(command string) string {
	s := strings.TrimSpace(strings.ReplaceAll(command, "\n", " "))
	if len(s) <= maxCommand {
		return s
	}
	cut := s[:maxCommand]
	for len(cut) > 0 && !utf8.ValidString(cut) {
		cut = cut[:len(cut)-1]
	}
	return cut + "…"
}
