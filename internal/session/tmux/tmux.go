// Package tmux implements rhost's persistent-session backend using remote tmux.
//
// A session is owned entirely by remote tmux and remote files, so it survives
// the CLI process and SSH disconnects (docs/ARCHITECTURE.md §12). rhost never
// holds a live PTY in local memory.
//
// On the remote host each session lives under
//
//	$RHOST_REMOTE_STATE/sessions/<id>/
//	    meta.json   discovery metadata (authoritative)
//	    pty.log     raw pane output, appended by `tmux pipe-pane`
//	    lock        flock target serialising writers
//
// The shell inside the pane is `bash --noprofile --norc -i` with an injected
// OSC 133 (FinalTerm) integration: the shell emits `ESC ] 133 ; D ; <exit> BEL`
// after every command, which gives a reliable command boundary and exit status
// without an in-band sentinel. Input echo is disabled so captured output is
// clean; `session attach` re-enables it for humans.
package tmux

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/shell"
)

// DefaultShell is the interactive shell launched inside a managed pane. The
// integration script needs bash (or zsh); we standardise on bash for v0.1.
const DefaultShell = "bash"

// Meta is the discovery metadata for a managed session (docs/ARCHITECTURE.md §13).
type Meta struct {
	SchemaVersion int    `json:"schema_version"`
	ID            string `json:"id"`
	Name          string `json:"name"`
	TmuxSession   string `json:"tmux_session"`
	CreatedAt     string `json:"created_at"`
	CreatedBy     string `json:"created_by"`
	InitialCwd    string `json:"initial_cwd,omitempty"`
	Shell         string `json:"shell"`
}

// NewMeta builds metadata for a new session.
func NewMeta(id, name, cwd, shellName string) Meta {
	return Meta{
		SchemaVersion: 1,
		ID:            id,
		Name:          name,
		TmuxSession:   TmuxName(id),
		CreatedAt:     time.Now().UTC().Format(time.RFC3339),
		CreatedBy:     "rhost",
		InitialCwd:    cwd,
		Shell:         shellName,
	}
}

// TmuxName is the namespaced tmux session name for a session id.
func TmuxName(id string) string { return "rhost_s_" + id }

// basePreamble defines the remote state root used by every script.
const basePreamble = "BASE=\"${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost}\"\n"

// tmuxPreflight fails fast (with a stable RHOST_ERR) when the remote lacks tmux.
// Without it, `tmux new-session` would fail silently and the readiness wait would
// surface a misleading SESSION_UNHEALTHY after roughly 30 seconds.
const tmuxPreflight = "command -v tmux >/dev/null 2>&1 || { echo RHOST_ERR=notmux; exit 0; }\n"

// flockPreflight distinguishes a missing flock (util-linux) from real lock
// contention. Without it `flock` failing with 127 would be reported as
// RHOST_ERR=locked, i.e. "another writer holds the lock", which is a lie.
const flockPreflight = "command -v flock >/dev/null 2>&1 || { echo RHOST_ERR=noflock; exit 0; }\n"

// nameOfFunc maps a meta.json path to its session name on stdout.
const nameOfFunc = `RHOST_NAME_OF() {
  sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1"
}
`

// resolveFunc is a shell helper that maps a user-supplied name-or-id to a
// session directory by scanning meta.json files. It keeps name resolution on
// the remote side so every command is a single SSH round-trip.
const resolveFunc = `RHOST_RESOLVE() {
  local want="$1" d id nm
  for d in "$BASE"/sessions/*/; do
    [ -f "$d/meta.json" ] || continue
    id=$(basename "$d")
    nm=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$d/meta.json")
    if [ "$id" = "$want" ] || { [ -n "$nm" ] && [ "$nm" = "$want" ]; }; then
      RHOST_DIR="$d"
      RHOST_ID="$id"
      RHOST_TMUX=$(sed -n 's/.*"tmux_session"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$d/meta.json")
      return 0
    fi
  done
  return 1
}
`

func b64(s string) string { return base64.StdEncoding.EncodeToString([]byte(s)) }

// integrationScript configures the pane's interactive bash. It is injected once
// over the pane's stdin (never written to disk). The final `: > ready` creates a
// readiness file; when it appears, every earlier line has run (including
// `stty -echo`), so the session is safe to use.
//
// The ready path is built from the same base expression the create script uses,
// expanded by the *pane's* shell (double quotes, not single quotes).
func integrationScript(id string) string {
	ready := `"${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost}/sessions/` + id + `/ready"`
	return `__rh_done() { printf '\033]133;D;%d\007' "$?"; }
__rh_exec() { printf '\033]133;C\007'; }
trap '__rh_exec' DEBUG
PROMPT_COMMAND=__rh_done
PS1='' ; PS2=''
stty -echo 2>/dev/null
bind 'set enable-bracketed-paste off' 2>/dev/null
: > ` + ready + "\n"
}

// CreateScript creates a session and blocks until its shell is ready. It prints
// RHOST_ERR=notready if the shell never came up.
func CreateScript(meta Meta, paneShellCmd string) string {
	dir := "$BASE/sessions/" + meta.ID
	metaJSON, _ := json.Marshal(meta)

	var b strings.Builder
	p := func(format string, a ...interface{}) { fmt.Fprintf(&b, format, a...) }

	b.WriteString(basePreamble)
	b.WriteString(tmuxPreflight)
	b.WriteString(nameOfFunc)
	// Reject a duplicate name before creating anything: resolving a name picks the
	// first matching meta.json, so two sessions sharing a name behave ambiguously.
	p("for d in \"$BASE\"/sessions/*/; do\n"+
		"  [ -f \"$d/meta.json\" ] || continue\n"+
		"  [ \"$(RHOST_NAME_OF \"$d/meta.json\")\" = %s ] && { echo RHOST_ERR=nameinuse; exit 0; }\n"+
		"done\n", shell.Quote(meta.Name))
	p("DIR=\"%s\"\n", dir)
	b.WriteString("LOG=\"$DIR/pty.log\"\n")
	b.WriteString("mkdir -p \"$DIR\" && chmod 700 \"$DIR\" 2>/dev/null\n")
	b.WriteString("rm -f \"$DIR/ready\" \"$LOG\"\n")

	tmux := shell.Quote(meta.TmuxSession)
	pane := shell.Quote(meta.TmuxSession + ":0.0")
	p("tmux kill-session -t %s 2>/dev/null\n", tmux)
	if meta.InitialCwd != "" {
		p("tmux new-session -d -s %s -x 220 -y 50 -c %s %s || { echo RHOST_ERR=newfailed; exit 0; }\n", tmux, shell.Quote(meta.InitialCwd), shell.Quote(paneShellCmd))
	} else {
		p("tmux new-session -d -s %s -x 220 -y 50 %s || { echo RHOST_ERR=newfailed; exit 0; }\n", tmux, shell.Quote(paneShellCmd))
	}
	p("tmux set-option -t %s history-limit 50000\n", tmux)
	// pipe-pane command is run by tmux via `sh -c`, so quote the path safely.
	b.WriteString("printf -v QPIPE 'cat >> %q' \"$LOG\"\n")
	p("tmux pipe-pane -t %s -o \"$QPIPE\"\n", pane)

	// Wait for the pane's foreground process to become bash. tmux runs the pane
	// command through the login shell, so an extra shell may front bash.
	p("i=0; while [ \"$i\" -lt 200 ]; do c=$(tmux display-message -p -t %s '#{pane_current_command}' 2>/dev/null); [ \"$c\" = bash ] && break; sleep 0.1; i=$((i+1)); done\n", pane)

	// Inject the integration script, then wait for the readiness file.
	p("printf '%%s' '%s' | base64 -d | tmux load-buffer -b rhost_int -\n", b64(integrationScript(meta.ID)))
	p("tmux paste-buffer -b rhost_int -t %s 2>/dev/null\n", pane)
	b.WriteString("tmux delete-buffer -b rhost_int 2>/dev/null\n")
	p("tmux send-keys -t %s Enter\n", pane)
	b.WriteString("i=0; while [ \"$i\" -lt 100 ]; do [ -f \"$DIR/ready\" ] && break; sleep 0.1; i=$((i+1)); done\n")
	b.WriteString("if [ ! -f \"$DIR/ready\" ]; then tmux kill-session -t " + tmux + " 2>/dev/null; rm -rf \"$DIR\"; echo RHOST_ERR=notready; exit 0; fi\n")
	b.WriteString("rm -f \"$DIR/ready\"\n")
	// Drop the bootstrap chatter (integration echo / first prompts) so `read`
	// from offset 0 starts clean. pipe-pane's `cat` appends, so truncation is safe.
	b.WriteString(": > \"$LOG\"\n")

	p("printf '%%s' '%s' | base64 -d > \"$DIR/meta.json\"\n", b64(string(metaJSON)))
	b.WriteString("chmod 600 \"$DIR/meta.json\" 2>/dev/null\n")
	b.WriteString("echo RHOST_OK=created\n")
	return b.String()
}

// markerScanFunc defines rh_window, the incremental completion-marker scan used
// by ExecScript. The globals it reads (`LOG`, `dpat`, `dlen`, `floor`, `scan`) are
// set by the calling script; it returns 0 when the marker appears in the window it
// covered, and always advances `scan` to the size it has actually read.
//
// The window re-reads the last dlen-1 bytes and never reaches back before
// `floor`. Both halves matter:
//
//   - without the overlap, a 9-byte marker written between one tick's size
//     measurement and that tick's read could be missed forever, turning a finished
//     command into a spurious REMOTE_COMMAND_TIMEOUT. Seen on a real host with
//     `bash -c 'exit 4'`: the marker was in the log and rhost never looked at it
//     again;
//   - without the floor, the overlap could re-read a *previous* command's marker
//     and report its exit code as this one's.
const markerScanFunc = `rh_window() {
  cur=$(wc -c < "$LOG" 2>/dev/null || echo 0)
  from=$((scan - dlen + 1))
  [ "$from" -lt "$floor" ] && from=$floor
  if [ "$cur" -ge "$from" ]; then
    tail -c +"$from" "$LOG" 2>/dev/null | head -c $((cur - from + 1)) | grep -aqF "$dpat" && { scan=$cur; return 0; }
  fi
  scan=$cur
  return 1
}
`

// ExecScript runs a command in an existing session and prints `RHOST_EXIT=<code>`
// followed by the base64 of the command's output.
//
// It serialises writers with flock, waits for the pane to be idle, records a log
// offset, pastes the command, then waits for the next OSC 133 D marker. On its
// own timeout it sends Ctrl-C (keeping the session usable) and prints
// RHOST_ERR=timeout.
func ExecScript(nameOrID, command string, timeout time.Duration) string {
	cmdB64 := b64(command)
	timeoutSec := int(timeout / time.Second)
	if timeoutSec < 1 {
		timeoutSec = 1
	}

	var b strings.Builder
	p := func(format string, a ...interface{}) { fmt.Fprintf(&b, format, a...) }

	b.WriteString(basePreamble)
	b.WriteString(resolveFunc)
	p("RHOST_RESOLVE %s || { echo RHOST_ERR=nosession; exit 0; }\n", shell.Quote(nameOrID))
	b.WriteString(tmuxPreflight)
	b.WriteString(flockPreflight)
	b.WriteString("DIR=\"$RHOST_DIR\"; LOG=\"$DIR/pty.log\"; LOCK=\"$DIR/lock\"; TMUX=\"$RHOST_TMUX\"\n")
	b.WriteString("LOG=$(printf '%s' \"$LOG\" | sed 's#/$##')\n") // trim trailing slash
	b.WriteString("exec 9>\"$LOCK\" || { echo RHOST_ERR=nosession; exit 0; }\n")
	b.WriteString("flock -w 30 9 || { echo RHOST_ERR=locked; exit 0; }\n")
	b.WriteString("tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n")

	// The D marker is an OSC 133 sequence terminated by BEL. `dpat` and `dlen` are
	// computed once and reused, and rh_window advances the scan by only as much as
	// it has actually *read* (re-reading the last marker-length bytes), so each poll
	// costs a small tail instead of re-scanning the log (quadratic on large output)
	// while a marker can never straddle two polls unnoticed.
	b.WriteString(markerScanFunc)
	b.WriteString("pre=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n")
	b.WriteString("tmux send-keys -t \"$TMUX:0.0\" 'stty -echo 2>/dev/null' Enter\n")
	b.WriteString("dpat=$(printf '\\033]133;D;'); dlen=${#dpat}\n")
	b.WriteString("floor=$((pre + 1)); scan=$floor; i=0; while [ \"$i\" -lt 50 ]; do rh_window && break; sleep 0.1; i=$((i+1)); done\n")
	b.WriteString("start=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n")

	// Paste the command as one compound command so it produces a single D.
	b.WriteString("cmd=$(printf '%s' '" + cmdB64 + "' | base64 -d)\n")
	b.WriteString("payload=\"{ $cmd\n}\"\n")
	b.WriteString("printf '%s' \"$payload\" | tmux load-buffer -b rhost_cmd -\n")
	b.WriteString("tmux paste-buffer -b rhost_cmd -t \"$TMUX:0.0\" 2>/dev/null\n")
	b.WriteString("tmux delete-buffer -b rhost_cmd 2>/dev/null\n")
	b.WriteString("tmux send-keys -t \"$TMUX:0.0\" Enter\n")

	// Wait for the next D marker after start, incrementally and with backoff, but
	// bail out early if the pane's shell dies (for example the command was `exit`).
	p("SECONDS=0; found=0; died=0; floor=$((start + 1)); scan=$floor; while [ \"$SECONDS\" -lt %d ]; do rh_window && { found=1; break; }; if ! tmux has-session -t \"$TMUX\" 2>/dev/null; then died=1; break; fi; if [ \"$SECONDS\" -ge 5 ]; then sleep 1; elif [ \"$SECONDS\" -ge 1 ]; then sleep 0.5; else sleep 0.1; fi; done\n", timeoutSec)
	b.WriteString("if [ \"$died\" = 1 ]; then echo RHOST_ERR=sessiondied; exit 0; fi\n")
	b.WriteString("if [ \"$found\" != 1 ]; then tmux send-keys -t \"$TMUX:0.0\" C-c 2>/dev/null; echo RHOST_ERR=timeout; exit 0; fi\n")

	// Locate the first D at/after start, emit exit code then the region before it.
	p("dline=$(tail -c +$((start+1)) \"$LOG\" | grep -aboF %s | head -1)\n", `"$dpat"`)
	b.WriteString("doff=${dline%%:*}\n")
	b.WriteString("abs=$((start + doff))\n")
	b.WriteString("seg=$(tail -c +$((abs+1)) \"$LOG\" | head -c 24)\n")
	// Take the digits that follow the marker *we located*: strip that exact prefix,
	// then keep the leading run of digits. A greedy sed over the whole 24-byte
	// window would read the *last* `;D;` in it, so a command whose marker sat next
	// to another one (an interrupt landing right after it) reported the wrong code.
	b.WriteString("tmp=${seg#\"$dpat\"}\n")
	b.WriteString("code=${tmp%%[!0-9]*}\n")
	b.WriteString("[ -n \"$code\" ] || code=-1\n")
	b.WriteString("echo \"RHOST_EXIT=$code\"\n")
	b.WriteString("tail -c +$((start+1)) \"$LOG\" | head -c $((abs - start)) | base64 -w0\n")
	b.WriteString("echo\n")
	return b.String()
}

// SendScript injects raw text (KIND=data) or a single key (KIND=key) into the
// pane. No completion boundary is expected; use this for REPLs, debuggers and
// control keys.
func SendScript(nameOrID, kind, payload string) string {
	var b strings.Builder
	p := func(format string, a ...interface{}) { fmt.Fprintf(&b, format, a...) }

	b.WriteString(basePreamble)
	b.WriteString(resolveFunc)
	p("RHOST_RESOLVE %s || { echo RHOST_ERR=nosession; exit 0; }\n", shell.Quote(nameOrID))
	b.WriteString(tmuxPreflight)
	b.WriteString("TMUX=\"$RHOST_TMUX\"\n")
	b.WriteString("tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n")
	if kind == "key" {
		p("tmux send-keys -t \"$TMUX:0.0\" %s\n", shell.Quote(payload))
	} else {
		p("printf '%%s' '%s' | base64 -d | tmux load-buffer -b rhost_send -\n", b64(payload))
		b.WriteString("tmux paste-buffer -b rhost_send -t \"$TMUX:0.0\" 2>/dev/null\n")
		b.WriteString("tmux delete-buffer -b rhost_send 2>/dev/null\n")
	}
	b.WriteString("echo RHOST_OK=sent\n")
	return b.String()
}

// ReadScript returns the raw pane log from byte offset `since`, base64-encoded,
// with explicit cursors so agents can poll incrementally.
func ReadScript(nameOrID string, since int, maxBytes int) string {
	if maxBytes <= 0 {
		maxBytes = 256 * 1024
	}
	if since < 0 {
		since = 0
	}

	var b strings.Builder
	p := func(format string, a ...interface{}) { fmt.Fprintf(&b, format, a...) }

	b.WriteString(basePreamble)
	b.WriteString(resolveFunc)
	p("RHOST_RESOLVE %s || { echo RHOST_ERR=nosession; exit 0; }\n", shell.Quote(nameOrID))
	b.WriteString("DIR=\"$RHOST_DIR\"; LOG=\"$DIR/pty.log\"\n")
	b.WriteString("LOG=$(printf '%s' \"$LOG\" | sed 's#/$##')\n")
	b.WriteString("size=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n")
	p("since=%d\n", since)
	b.WriteString("[ \"$since\" -gt \"$size\" ] && since=$size\n")
	p("inc=$(tail -c +$((since+1)) \"$LOG\" 2>/dev/null | head -c %d | wc -c)\n", maxBytes)
	b.WriteString("echo \"RHOST_FROM=$since\"\n")
	b.WriteString("echo \"RHOST_NEXT=$((since + inc))\"\n")
	b.WriteString("echo \"RHOST_SIZE=$size\"\n")
	b.WriteString("tail -c +$((since+1)) \"$LOG\" 2>/dev/null | head -c \"$inc\" | base64 -w0\n")
	b.WriteString("echo\n")
	return b.String()
}

// ListScript prints one RHOST_META line per session, each carrying the id, the
// liveness of its tmux session, and the base64 of meta.json.
func ListScript() string {
	return basePreamble + tmuxPreflight + `[ -d "$BASE/sessions" ] || exit 0
for d in "$BASE"/sessions/*/; do
  [ -f "$d/meta.json" ] || continue
  id=$(basename "$d")
  tmuxname=$(sed -n 's/.*"tmux_session"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$d/meta.json")
  alive=no
  if [ -n "$tmuxname" ] && tmux has-session -t "$tmuxname" 2>/dev/null; then alive=yes; fi
  meta=$(base64 -w0 < "$d/meta.json")
  printf 'RHOST_META\t%s\t%s\t%s\n' "$id" "$alive" "$meta"
done
`
}

// CloseScript kills the tmux session and removes its state directory.
func CloseScript(nameOrID string) string {
	var b strings.Builder
	p := func(format string, a ...interface{}) { fmt.Fprintf(&b, format, a...) }
	b.WriteString(basePreamble)
	b.WriteString(resolveFunc)
	p("RHOST_RESOLVE %s || { echo RHOST_ERR=nosession; exit 0; }\n", shell.Quote(nameOrID))
	b.WriteString("tmux kill-session -t \"$RHOST_TMUX\" 2>/dev/null\n")
	b.WriteString("rm -rf \"$RHOST_DIR\"\n")
	b.WriteString("echo RHOST_OK=closed\n")
	return b.String()
}

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
	Output   string
	ExitCode int
	Err      string // non-empty for a helper-level problem (timeout, nosession, …)
}

// ParseExec parses ExecScript output.
func ParseExec(stdout string) ExecOutcome {
	out := ExecOutcome{ExitCode: -1}
	if err := fieldLine(stdout, "RHOST_ERR="); err != "" {
		out.Err = err
		return out
	}
	lines := strings.Split(stdout, "\n")
	var b64buf strings.Builder
	for _, line := range lines {
		if strings.HasPrefix(line, "RHOST_EXIT=") {
			fmt.Sscanf(strings.TrimPrefix(line, "RHOST_EXIT="), "%d", &out.ExitCode)
			continue
		}
		if line == "" || strings.HasPrefix(line, "RHOST_") {
			continue
		}
		b64buf.WriteString(strings.TrimSpace(line))
	}
	if raw, err := base64.StdEncoding.DecodeString(b64buf.String()); err == nil {
		out.Output = string(raw)
	}
	return out
}

// ReadOutcome is the parsed result of ReadScript.
type ReadOutcome struct {
	From  int
	Next  int
	Size  int
	Data  []byte
	Error string
}

// ParseRead parses ReadScript output.
func ParseRead(stdout string) ReadOutcome {
	out := ReadOutcome{}
	if err := fieldLine(stdout, "RHOST_ERR="); err != "" {
		out.Error = err
		return out
	}
	var b64buf strings.Builder
	for _, line := range strings.Split(stdout, "\n") {
		switch {
		case strings.HasPrefix(line, "RHOST_FROM="):
			fmt.Sscanf(strings.TrimPrefix(line, "RHOST_FROM="), "%d", &out.From)
		case strings.HasPrefix(line, "RHOST_NEXT="):
			fmt.Sscanf(strings.TrimPrefix(line, "RHOST_NEXT="), "%d", &out.Next)
		case strings.HasPrefix(line, "RHOST_SIZE="):
			fmt.Sscanf(strings.TrimPrefix(line, "RHOST_SIZE="), "%d", &out.Size)
		case strings.HasPrefix(line, "RHOST_"):
			// ignore
		default:
			b64buf.WriteString(strings.TrimSpace(line))
		}
	}
	if raw, err := base64.StdEncoding.DecodeString(b64buf.String()); err == nil {
		out.Data = raw
	}
	return out
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
