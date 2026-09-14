package tmux

import (
	"encoding/base64"
	"encoding/json"
	"sort"
	"strconv"
	"strings"
)

// ListEntry is one row parsed from ListScript.
type ListEntry struct {
	ID    string
	Alive bool
	Meta  Meta
}

// ParseList parses ListScript output.
func ParseList(stdout string) []ListEntry {
	var out []ListEntry
	for _, line := range strings.Split(stdout, "\n") {
		if !strings.HasPrefix(line, "RHOST_META\t") {
			continue
		}
		parts := strings.SplitN(line, "\t", 4)
		if len(parts) != 4 {
			continue
		}
		raw, err := base64.StdEncoding.DecodeString(strings.TrimSpace(parts[3]))
		if err != nil {
			continue
		}
		var m Meta
		if err := json.Unmarshal(raw, &m); err != nil {
			continue
		}
		out = append(out, ListEntry{ID: parts[1], Alive: parts[2] == "yes", Meta: m})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// ExecOutcome is the parsed result of ExecScript.
type ExecOutcome struct {
	SessionID string
	Output    string
	ExitCode  int
	Err       string // non-empty for a helper-level problem (timeout, nosession, …)
	// Foreground is the pane's foreground command when the helper refused with
	// "busy". It names what is holding the terminal, so the answer can say so.
	Foreground string
	// Recovered says the pane came back to a prompt after the helper interrupted a
	// timed-out command. It is only ever meaningful together with Err == "timeout",
	// and "not recovered" is a different answer from "timed out": the session may
	// still be holding a running program.
	Recovered bool
}

// ParseExec parses ExecScript output.
func ParseExec(stdout, expectedToken string) ExecOutcome {
	out := ExecOutcome{ExitCode: -1}
	if err := fieldLine(stdout, "RHOST_ERR="); err != "" {
		out.Err = err
		out.SessionID = fieldLine(stdout, "RHOST_ID=")
		out.Foreground = fieldLine(stdout, "RHOST_FG=")
		out.Recovered = fieldLine(stdout, "RHOST_RECOVERED=") == "1"
		return out
	}
	var tokens, exits, outputs []string
	for _, line := range strings.Split(stdout, "\n") {
		switch {
		case strings.HasPrefix(line, "RHOST_TOKEN="):
			tokens = append(tokens, strings.TrimPrefix(line, "RHOST_TOKEN="))
		case strings.HasPrefix(line, "RHOST_EXIT="):
			exits = append(exits, strings.TrimPrefix(line, "RHOST_EXIT="))
		case strings.HasPrefix(line, "RHOST_OUTPUT="):
			outputs = append(outputs, strings.TrimPrefix(line, "RHOST_OUTPUT="))
		case strings.HasPrefix(line, "RHOST_ID="):
			if out.SessionID != "" {
				out.Err = "protocol"
				return out
			}
			out.SessionID = strings.TrimPrefix(line, "RHOST_ID=")
		case line == "":
		default:
			out.Err = "protocol"
			return out
		}
	}
	if expectedToken == "" || out.SessionID == "" || len(tokens) != 1 || tokens[0] != expectedToken || len(exits) != 1 || len(outputs) != 1 {
		out.Err = "protocol"
		return out
	}
	code, err := strconv.Atoi(exits[0])
	if err != nil || code < 0 || code > 255 || strconv.Itoa(code) != exits[0] {
		out.Err = "protocol"
		return out
	}
	raw, err := base64.StdEncoding.DecodeString(outputs[0])
	if err != nil {
		out.Err = "protocol"
		return out
	}
	out.ExitCode = code
	out.Output = string(raw)
	return out
}

// ReadOutcome is the parsed result of ReadScript.
type ReadOutcome struct {
	SessionID string
	From      int
	Next      int
	Size      int
	Data      []byte
	Error     string
}

// ParseRead parses ReadScript output.
func ParseRead(stdout string) ReadOutcome {
	out := ReadOutcome{}
	if err := fieldLine(stdout, "RHOST_ERR="); err != "" {
		out.Error = err
		out.SessionID = fieldLine(stdout, "RHOST_ID=")
		return out
	}
	seen := map[string]bool{}
	var b64buf strings.Builder
	for _, line := range strings.Split(stdout, "\n") {
		switch {
		case strings.HasPrefix(line, "RHOST_ID="):
			if out.SessionID != "" {
				out.Error = "protocol"
				return out
			}
			out.SessionID = strings.TrimPrefix(line, "RHOST_ID=")
		case strings.HasPrefix(line, "RHOST_FROM="):
			if !parseReadField(line, "RHOST_FROM=", &out.From, seen) {
				out.Error = "protocol"
				return out
			}
		case strings.HasPrefix(line, "RHOST_NEXT="):
			if !parseReadField(line, "RHOST_NEXT=", &out.Next, seen) {
				out.Error = "protocol"
				return out
			}
		case strings.HasPrefix(line, "RHOST_SIZE="):
			if !parseReadField(line, "RHOST_SIZE=", &out.Size, seen) {
				out.Error = "protocol"
				return out
			}
		case strings.HasPrefix(line, "RHOST_"):
			out.Error = "protocol"
			return out
		default:
			b64buf.WriteString(strings.TrimSpace(line))
		}
	}
	if out.SessionID == "" || !seen["RHOST_FROM="] || !seen["RHOST_NEXT="] || !seen["RHOST_SIZE="] {
		out.Error = "protocol"
		return out
	}
	if out.From < 0 || out.Next < out.From || out.Size < out.Next {
		out.Error = "protocol"
		return out
	}
	raw, err := base64.StdEncoding.DecodeString(b64buf.String())
	if err != nil {
		out.Error = "protocol"
		return out
	}
	out.Data = raw
	return out
}

func parseReadField(line, prefix string, dst *int, seen map[string]bool) bool {
	if seen[prefix] {
		return false
	}
	value := strings.TrimPrefix(line, prefix)
	n, err := strconv.Atoi(value)
	if err != nil {
		return false
	}
	*dst = n
	seen[prefix] = true
	return true
}

// fieldLine returns the value of `key` if a line starts with it, else "".
func fieldLine(stdout, key string) string {
	for _, line := range strings.Split(stdout, "\n") {
		if strings.HasPrefix(line, key) {
			return strings.TrimSpace(strings.TrimPrefix(line, key))
		}
	}
	return ""
}
