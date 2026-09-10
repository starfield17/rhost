package detached

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
)

// HelperErr is the resolved value of an RHOST_ERR= line emitted by a helper
// script. Codes are stable and mapped to errs.Code in the app layer.
type HelperErr string

// NotFound is the helper error for a vanished job directory.
const NotFound HelperErr = "nojob"

// factsFromLines parses RHOST_* fact lines into Facts. id comes either from
// RHOST_JOB= (individual scripts) or from the RHOST_META tab row (list).
func factsFromLines(lines []string, id string) (Facts, bool) {
	f := Facts{ID: id, ExitCode: -1}
	ok := false
	var metaB64 string
	for _, line := range lines {
		switch {
		case strings.HasPrefix(line, "RHOST_JOB="):
			f.ID = strings.TrimSpace(strings.TrimPrefix(line, "RHOST_JOB="))
			ok = true
		case strings.HasPrefix(line, "RHOST_PID="):
			f.PID, _ = strconv.Atoi(strings.TrimSpace(strings.TrimPrefix(line, "RHOST_PID=")))
		case strings.HasPrefix(line, "RHOST_ALIVE="):
			f.Alive = strings.TrimSpace(strings.TrimPrefix(line, "RHOST_ALIVE=")) == "yes"
		case strings.HasPrefix(line, "RHOST_EXIT="):
			// Strict on purpose: an unreadable or empty exit_code stays -1 ("no
			// code recorded"). Atoi("") failing into a 0 would report a job that
			// never finished our way as `exited`.
			if n, err := strconv.Atoi(strings.TrimSpace(strings.TrimPrefix(line, "RHOST_EXIT="))); err == nil {
				f.ExitCode = n
			}
			ok = true
		case strings.HasPrefix(line, "RHOST_STOPPED="):
			f.Stopped = strings.TrimSpace(strings.TrimPrefix(line, "RHOST_STOPPED=")) == "yes"
		case strings.HasPrefix(line, "RHOST_FINISHED_AT="):
			f.FinishedAt = strings.TrimSpace(strings.TrimPrefix(line, "RHOST_FINISHED_AT="))
		case strings.HasPrefix(line, "RHOST_META="):
			metaB64 = strings.TrimSpace(strings.TrimPrefix(line, "RHOST_META="))
		}
	}
	if metaB64 != "" {
		if raw, err := base64.StdEncoding.DecodeString(metaB64); err == nil {
			var m Meta
			if err := json.Unmarshal(raw, &m); err == nil {
				f.Meta = &m
			}
		}
	}
	return f, ok
}

// helperErr extracts an RHOST_ERR= code, or "" when none is present. The
// helper scripts print exactly one such line (possibly empty) on the first
// line of stdout when they fail fast.
func helperErr(stdout string) HelperErr {
	for _, line := range strings.Split(stdout, "\n") {
		if strings.HasPrefix(line, "RHOST_ERR=") {
			return HelperErr(strings.TrimSpace(strings.TrimPrefix(line, "RHOST_ERR=")))
		}
	}
	return ""
}

// ErrOf exposes the same reading to the app layer, which has to tell "no such
// job" (resolve the handle as a name and retry) from every other helper failure
// (report it as-is).
func ErrOf(stdout string) HelperErr { return helperErr(stdout) }

// ParseStatus parses StatusScript output into Facts. A non-empty HelperErr
// means the helper refused (no job, missing dependency, start failure).
func ParseStatus(stdout string) (Facts, HelperErr) {
	if err := helperErr(stdout); err != "" {
		return Facts{}, err
	}
	f, ok := factsFromLines(strings.Split(stdout, "\n"), "")
	if !ok {
		return Facts{}, HelperErr("malformed")
	}
	return f, ""
}

// ParseStart parses StartScript output. The start response carries the same
// fact lines as a status response, so the same parser applies; ok is false
// when the launch reported a non-RHOST form.
func ParseStart(stdout string) (Facts, HelperErr) {
	if err := helperErr(stdout); err != "" {
		return Facts{}, err
	}
	f, ok := factsFromLines(strings.Split(stdout, "\n"), "")
	if !ok {
		return Facts{}, HelperErr("malformed")
	}
	return f, ""
}

// ListEntry is one job row parsed from ListScript output.
type ListEntry struct {
	Facts Facts
}

// ParseList parses ListScript output into one Facts per RHOST_META tab row.
//
// Row layout: id, pid, alive, exit, stopped, finished_at, meta(base64). The pid
// is in the row so `job list` cannot claim pid 0 for a job that is running.
func ParseList(stdout string) []Facts {
	var out []Facts
	for _, line := range strings.Split(stdout, "\n") {
		if !strings.HasPrefix(line, "RHOST_META\t") {
			continue
		}
		parts := strings.SplitN(line, "\t", 8)
		if len(parts) != 8 {
			continue
		}
		f := Facts{ID: parts[1], Alive: parts[3] == "yes", ExitCode: -1}
		if n, err := strconv.Atoi(strings.TrimSpace(parts[2])); err == nil {
			f.PID = n
		}
		if n, err := strconv.Atoi(strings.TrimSpace(parts[4])); err == nil {
			f.ExitCode = n
		}
		f.Stopped = parts[5] == "yes"
		f.FinishedAt = parts[6]
		if metaB64 := strings.TrimSpace(parts[7]); metaB64 != "" {
			if raw, err := base64.StdEncoding.DecodeString(metaB64); err == nil {
				var m Meta
				if err := json.Unmarshal(raw, &m); err == nil {
					f.Meta = &m
				}
			}
		}
		out = append(out, f)
	}
	return out
}

// LogOutcome is the parsed result of LogsScript.
type LogOutcome struct {
	From int
	Size int
	Next int
	Data []byte
}

// ParseLogs parses LogsScript output. The data is the base64-decoded chunk.
func ParseLogs(stdout string) (LogOutcome, HelperErr) {
	if err := helperErr(stdout); err != "" {
		return LogOutcome{}, err
	}
	out := LogOutcome{}
	var b64buf strings.Builder
	for _, line := range strings.Split(stdout, "\n") {
		switch {
		case strings.HasPrefix(line, "RHOST_SINCE="):
			out.From, _ = strconv.Atoi(strings.TrimSpace(strings.TrimPrefix(line, "RHOST_SINCE=")))
		case strings.HasPrefix(line, "RHOST_SIZE="):
			out.Size, _ = strconv.Atoi(strings.TrimSpace(strings.TrimPrefix(line, "RHOST_SIZE=")))
		case strings.HasPrefix(line, "RHOST_NEXT="):
			out.Next, _ = strconv.Atoi(strings.TrimSpace(strings.TrimPrefix(line, "RHOST_NEXT=")))
		case strings.HasPrefix(line, "RHOST_"):
			// skip
		default:
			b64buf.WriteString(strings.TrimSpace(line))
		}
	}
	raw, err := base64.StdEncoding.DecodeString(b64buf.String())
	if err == nil {
		out.Data = raw
	}
	if out.Next == 0 && out.Size > 0 {
		return out, HelperErr("malformed")
	}
	return out, ""
}

// ErrString renders a HelperErr for diagnostics.
func (e HelperErr) ErrString() string { return fmt.Sprintf("job helper error: %s", string(e)) }
