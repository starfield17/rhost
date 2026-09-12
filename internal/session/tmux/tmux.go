// Package tmux implements rhost's persistent-session backend using remote tmux.
//
// A session is owned entirely by remote tmux and remote files, so it survives
// the CLI process and SSH disconnects (docs/architecture/persistent-work.md). rhost never
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
	"strconv"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/shell"
)

// DefaultShell is the interactive shell launched inside a managed pane. The
// integration script needs bash (or zsh); we standardise on bash for the current backend.
const DefaultShell = "bash"

// Meta is the discovery metadata for a managed session (docs/architecture/persistent-work.md).
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

// shellOfFunc reads the managed shell out of a session's metadata, so the
// foreground check compares against what this session was actually created with
// rather than against a hardcoded name.
const shellOfFunc = `RHOST_SHELL_OF() {
  sed -n 's/.*"shell"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1"
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
	b.WriteString("command -v flock >/dev/null 2>&1 || { echo RHOST_ERR=noflock; exit 0; }\n")
	b.WriteString(nameOfFunc)
	b.WriteString("mkdir -p \"$BASE/sessions\" && chmod 700 \"$BASE/sessions\" 2>/dev/null || { echo RHOST_ERR=newfailed; exit 0; }\n")
	b.WriteString("exec 8> \"$BASE/sessions/.create.lock\"\n")
	b.WriteString("flock -w 30 8 || { echo RHOST_ERR=locked; exit 0; }\n")
	// Reject a duplicate name before creating anything: resolving a name picks the
	// first matching meta.json, so two sessions sharing a name behave ambiguously.
	p("for d in \"$BASE\"/sessions/*/; do\n"+
		"  [ -f \"$d/meta.json\" ] || continue\n"+
		"  [ \"$(RHOST_NAME_OF \"$d/meta.json\")\" = %s ] && { echo RHOST_ERR=nameinuse; exit 0; }\n"+
		"done\n", shell.Quote(meta.Name))
	p("DIR=\"%s\"\n", dir)
	b.WriteString("LOG=\"$DIR/pty.log\"\n")
	b.WriteString("mkdir -p \"$DIR\" && chmod 700 \"$DIR\" 2>/dev/null || { rm -rf \"$DIR\"; echo RHOST_ERR=newfailed; exit 0; }\n")
	b.WriteString("rm -f \"$DIR/ready\" \"$LOG\"\n")

	tmux := shell.Quote(meta.TmuxSession)
	pane := shell.Quote(meta.TmuxSession + ":0.0")
	b.WriteString("created=notyet; INTBUF=rhost_int_$$\n")
	b.WriteString("cleanup() { tmux delete-buffer -b \"$INTBUF\" 2>/dev/null; if [ \"$created\" = yes ]; then tmux kill-session -t " + tmux + " 2>/dev/null; rm -rf \"$DIR\"; fi; }\n")
	b.WriteString("trap cleanup EXIT\n")
	b.WriteString("trap 'exit 1' HUP INT TERM\n")
	p("tmux kill-session -t %s 2>/dev/null\n", tmux)
	if meta.InitialCwd != "" {
		p("tmux new-session -d -s %s -x 220 -y 50 -c %s %s || { echo RHOST_ERR=newfailed; exit 0; }\n", tmux, shell.PathQuote(meta.InitialCwd), shell.Quote(paneShellCmd))
	} else {
		p("tmux new-session -d -s %s -x 220 -y 50 %s || { echo RHOST_ERR=newfailed; exit 0; }\n", tmux, shell.Quote(paneShellCmd))
	}
	b.WriteString("created=yes\n")
	p("tmux set-option -t %s history-limit 50000\n", tmux)
	// pipe-pane command is run by tmux via `sh -c`, so quote the path safely.
	b.WriteString("printf -v QPIPE 'cat >> %q' \"$LOG\"\n")
	p("tmux pipe-pane -t %s -o \"$QPIPE\"\n", pane)

	// Wait for the pane's foreground process to become bash. tmux runs the pane
	// command through the login shell, so an extra shell may front bash.
	p("i=0; while [ \"$i\" -lt 200 ]; do c=$(tmux display-message -p -t %s '#{pane_current_command}' 2>/dev/null); [ \"$c\" = bash ] && break; sleep 0.1; i=$((i+1)); done\n", pane)

	// Inject the integration script, then wait for the readiness file.
	p("printf '%%s' '%s' | base64 -d | tmux load-buffer -b \"$INTBUF\" -\n", b64(integrationScript(meta.ID)))
	p("tmux paste-buffer -b \"$INTBUF\" -t %s 2>/dev/null\n", pane)
	b.WriteString("tmux delete-buffer -b \"$INTBUF\" 2>/dev/null\n")
	p("tmux send-keys -t %s Enter\n", pane)
	b.WriteString("i=0; while [ \"$i\" -lt 100 ]; do [ -f \"$DIR/ready\" ] && break; sleep 0.1; i=$((i+1)); done\n")
	b.WriteString("if [ ! -f \"$DIR/ready\" ]; then tmux kill-session -t " + tmux + " 2>/dev/null; rm -rf \"$DIR\"; echo RHOST_ERR=notready; exit 0; fi\n")
	b.WriteString("rm -f \"$DIR/ready\"\n")
	// Drop the bootstrap chatter (integration echo / first prompts) so `read`
	// from offset 0 starts clean. pipe-pane's `cat` appends, so truncation is safe.
	b.WriteString(": > \"$LOG\"\n")

	p("printf '%%s' '%s' | base64 -d > \"$DIR/meta.json\" || { echo RHOST_ERR=newfailed; exit 0; }\n", b64(string(metaJSON)))
	b.WriteString("chmod 600 \"$DIR/meta.json\" 2>/dev/null || { echo RHOST_ERR=newfailed; exit 0; }\n")
	b.WriteString("created=no\n")
	b.WriteString("trap - EXIT HUP INT TERM\n")
	b.WriteString("echo RHOST_OK=created\n")
	return b.String()
}

// markerScanFunc defines rh_window, the incremental completion-marker scan used
// by ExecScript. The globals it reads (`LOG`, `dpat`, `dlen`, `floor`, `scan`) are
// set by the calling script; it returns 0 when the marker appears in the window it
// covered, and always advances `scan` to a size it has actually read.
//
// The load-bearing property is the order: measure the log size, then read exactly
// that window, then advance to it. The old loop read first and measured afterwards,
// so the offset could jump past bytes nobody had read — and a marker inside that
// band was never inside any later window, which turned a finished command into a
// spurious REMOTE_COMMAND_TIMEOUT.
//
// Which of the two defences closes the gap was measured on a real remote host,
// three runs of TestLiveSession each: read-then-measure with no re-read failed 3/3;
// measure-then-read with no re-read passed 3/3; read-then-measure *with* the re-read
// passed 3/3. Either half is sufficient on its own, so both are kept — and the
// re-read is what the `floor` clamp is for: it lets the window overlap backwards
// without ever reaching into a previous command's marker and reporting its exit
// code as this one's.
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

// ExecScript runs a command in an existing session. Its result is accepted only
// when the pane emits a completion marker carrying token and a valid shell exit
// status; the generic OSC 133 D marker is used only to prove prompt readiness.
//
// It serialises writers with a non-blocking flock: another writer holding the
// lock is SESSION_UNHEALTHY immediately (retryable), not a 30s queue. Then it
// waits for the pane to be idle, records a log offset, pastes the command, and
// waits for the next OSC 133 D marker. On its own timeout it sends Ctrl-C
// (keeping the session usable) and prints RHOST_ERR=timeout.
//
// Before any of that it refuses to run at all unless the pane's foreground
// process is the managed shell: a command is pasted into the terminal, and a
// terminal owned by a REPL or a debugger would execute it in the wrong place.
// The refusal is its own code (busy, with the foreground command in RHOST_FG)
// because it is not a failure of the session — it is the caller asking the wrong
// tool for the job.
func ExecScript(nameOrID, command string, timeout time.Duration, token string) string {
	payload := "{\n" + command + "\ncommand printf '\\033]133;R;" + token + ";%d\\007' \"$?\"\n}"
	cmdB64 := b64(payload)
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
	b.WriteString("flock -n 9 || { echo RHOST_ERR=locked; exit 0; }\n")
	b.WriteString("tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n")
	b.WriteString("BUF=rhost_cmd_$$\n")
	b.WriteString("cleanup() { tmux delete-buffer -b \"$BUF\" 2>/dev/null; }\n")
	b.WriteString("trap cleanup EXIT; trap 'exit 1' HUP INT TERM\n")

	// Everything below types into the pane's terminal. That is only safe while
	// the managed shell owns the foreground: after `session send --data 'python\n'`
	// the pane is a REPL, and a pasted command (or the stty probe) would be read
	// by *it*, not by a shell (docs/architecture/persistent-work.md). So exec fails closed and
	// names what it found, leaving `session send`/`session read` as the honest way
	// to drive a program, and `session recover` as the explicit repair path.
	b.WriteString(shellOfFunc)
	b.WriteString("want=$(RHOST_SHELL_OF \"$DIR/meta.json\" 2>/dev/null)\n")
	b.WriteString("[ -n \"$want\" ] || want=" + shell.Quote(DefaultShell) + "\n")
	b.WriteString("fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n")
	b.WriteString("if [ -z \"$fg\" ]; then echo RHOST_ERR=unknownfg; exit 0; fi\n")
	b.WriteString("if [ \"$fg\" != \"$want\" ]; then echo \"RHOST_FG=$fg\"; echo RHOST_ERR=busy; exit 0; fi\n")

	// The D marker is an OSC 133 sequence terminated by BEL. `dpat` and `dlen` are
	// computed once and reused, and rh_window advances the scan by only as much as
	// it has actually *read* (re-reading the last marker-length bytes), so each poll
	// costs a small tail instead of re-scanning the log (quadratic on large output)
	// while a marker can never straddle two polls unnoticed.
	b.WriteString(markerScanFunc)
	b.WriteString("pre=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n")
	b.WriteString("TTY=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_tty}' 2>/dev/null)\n")
	b.WriteString("[ -n \"$TTY\" ] || { echo RHOST_ERR=notty; exit 0; }\n")
	b.WriteString("stty -echo < \"$TTY\" 2>/dev/null || { echo RHOST_ERR=notty; exit 0; }\n")
	b.WriteString("dpat=$(printf '\\033]133;D;'); dlen=${#dpat}\n")
	b.WriteString("tmux send-keys -t \"$TMUX:0.0\" Enter\n")
	b.WriteString("floor=$((pre + 1)); scan=$floor; i=0; while [ \"$i\" -lt 50 ]; do rh_window && break; sleep 0.1; i=$((i+1)); done\n")
	b.WriteString("[ \"$i\" -lt 50 ] || { echo RHOST_ERR=notready; exit 0; }\n")
	b.WriteString("fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n")
	b.WriteString("if [ -z \"$fg\" ]; then echo RHOST_ERR=unknownfg; exit 0; fi\n")
	b.WriteString("if [ \"$fg\" != \"$want\" ]; then echo \"RHOST_FG=$fg\"; echo RHOST_ERR=busy; exit 0; fi\n")
	b.WriteString("start=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n")

	// Paste the command as one compound command so it produces a single D.
	b.WriteString("cmd=$(printf '%s' '" + cmdB64 + "' | base64 -d)\n")
	b.WriteString("printf '%s' \"$cmd\" | tmux load-buffer -b \"$BUF\" -\n")
	b.WriteString("tmux paste-buffer -b \"$BUF\" -t \"$TMUX:0.0\" 2>/dev/null\n")
	b.WriteString("tmux delete-buffer -b \"$BUF\" 2>/dev/null\n")
	b.WriteString("tmux send-keys -t \"$TMUX:0.0\" Enter\n")
	b.WriteString("rpat=$(printf '\\033]133;R;" + token + ";'); dpat=$rpat; dlen=${#dpat}\n")

	// Wait for the next D marker after start, incrementally and with backoff, but
	// bail out early if the pane's shell dies (for example the command was `exit`).
	p("SECONDS=0; found=0; died=0; floor=$((start + 1)); scan=$floor; while [ \"$SECONDS\" -lt %d ]; do rh_window && { found=1; break; }; if ! tmux has-session -t \"$TMUX\" 2>/dev/null; then died=1; break; fi; if [ \"$SECONDS\" -ge 5 ]; then sleep 1; elif [ \"$SECONDS\" -ge 1 ]; then sleep 0.5; else sleep 0.1; fi; done\n", timeoutSec)
	b.WriteString("if [ \"$died\" = 1 ]; then echo RHOST_ERR=sessiondied; exit 0; fi\n")
	// A command that outlived its deadline is interrupted, and then rhost *proves*
	// the shell is usable again before handing the session back: Ctrl-C, followed by
	// up to 5s waiting for the next command boundary. RHOST_RECOVERED is that proof.
	// It is reported rather than assumed, because a Ctrl-C the pane ignored leaves a
	// program holding the session — and the next caller must not find that out by
	// sending a command into it.
	b.WriteString("if [ \"$found\" != 1 ]; then\n")
	b.WriteString("  tmux send-keys -t \"$TMUX:0.0\" C-c 2>/dev/null\n")
	b.WriteString("  recovered=0; dpat=$(printf '\\033]133;D;'); dlen=${#dpat}; floor=$((scan + 1)); scan=$floor; i=0\n")
	b.WriteString("  while [ \"$i\" -lt 50 ]; do rh_window && { recovered=1; break; }; sleep 0.1; i=$((i+1)); done\n")
	b.WriteString("  fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n")
	b.WriteString("  [ \"$fg\" = \"$want\" ] || recovered=0\n")
	b.WriteString("  echo \"RHOST_RECOVERED=$recovered\"\n")
	b.WriteString("  echo RHOST_ERR=timeout\n")
	b.WriteString("  exit 0\n")
	b.WriteString("fi\n")

	// Locate this invocation's token marker, validate its code and then emit the
	// exact three-field helper result. ParseExec independently validates it again.
	p("dline=$(tail -c +$((start+1)) \"$LOG\" | grep -aboF %s | head -1)\n", `"$rpat"`)
	b.WriteString("doff=${dline%%:*}\n")
	b.WriteString("abs=$((start + doff))\n")
	b.WriteString("seg=$(tail -c +$((abs+1)) \"$LOG\" | head -c $((dlen + 8)))\n")
	// Take the digits that follow the marker *we located*: strip that exact prefix,
	// then keep the leading run of digits. A greedy sed over the whole 24-byte
	// window would read the *last* `;D;` in it, so a command whose marker sat next
	// to another one (an interrupt landing right after it) reported the wrong code.
	b.WriteString("tmp=${seg#\"$rpat\"}\n")
	b.WriteString("code=${tmp%%[!0-9]*}\n")
	b.WriteString("bel=$(printf '\\007'); after=${tmp#\"$code\"}\n")
	b.WriteString("case \"$code\" in ''|*[!0-9]*) echo RHOST_ERR=protocol; exit 0;; esac\n")
	b.WriteString("[ \"$code\" -le 255 ] 2>/dev/null || { echo RHOST_ERR=protocol; exit 0; }\n")
	b.WriteString("[ \"${after#\"$bel\"}\" != \"$after\" ] || { echo RHOST_ERR=protocol; exit 0; }\n")
	b.WriteString("echo \"RHOST_TOKEN=" + token + "\"\n")
	b.WriteString("echo \"RHOST_EXIT=$code\"\n")
	b.WriteString("printf 'RHOST_OUTPUT='\n")
	b.WriteString("tail -c +$((start+1)) \"$LOG\" | head -c $((abs - start)) | base64 -w0\n")
	b.WriteString("printf '\\n'\n")
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
	b.WriteString(flockPreflight)
	b.WriteString("DIR=\"$RHOST_DIR\"; TMUX=\"$RHOST_TMUX\"; LOCK=\"$DIR/lock\"\n")
	b.WriteString("exec 9>\"$LOCK\" || { echo RHOST_ERR=nosession; exit 0; }\n")
	b.WriteString("flock -n 9 || { echo RHOST_ERR=locked; exit 0; }\n")
	b.WriteString("tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n")
	b.WriteString("BUF=rhost_send_$$\n")
	b.WriteString("cleanup() { tmux delete-buffer -b \"$BUF\" 2>/dev/null; }\n")
	b.WriteString("trap cleanup EXIT; trap 'exit 1' HUP INT TERM\n")
	b.WriteString("TTY=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_tty}' 2>/dev/null)\n")
	b.WriteString("[ -n \"$TTY\" ] || { echo RHOST_ERR=notty; exit 0; }\n")
	b.WriteString("stty -echo < \"$TTY\" 2>/dev/null || { echo RHOST_ERR=notty; exit 0; }\n")
	if kind == "key" {
		p("tmux send-keys -t \"$TMUX:0.0\" %s\n", shell.Quote(payload))
	} else {
		p("printf '%%s' '%s' | base64 -d | tmux load-buffer -b \"$BUF\" -\n", b64(payload))
		b.WriteString("tmux paste-buffer -b \"$BUF\" -t \"$TMUX:0.0\" 2>/dev/null\n")
		b.WriteString("tmux delete-buffer -b \"$BUF\" 2>/dev/null\n")
		if kind == "data-enter" {
			b.WriteString("tmux send-keys -t \"$TMUX:0.0\" Enter\n")
		}
	}
	b.WriteString("echo RHOST_OK=sent\n")
	return b.String()
}

// RecoverScript interrupts the pane under the same writer lock as exec/send,
// then proves both that the managed shell owns the foreground and that its
// prompt emitted a fresh readiness marker. A REPL that catches Ctrl-C remains
// busy and is reported without sending it an exit command.
func RecoverScript(nameOrID string, timeout time.Duration) string {
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
	b.WriteString("DIR=\"$RHOST_DIR\"; LOG=\"${RHOST_DIR%/}/pty.log\"; LOCK=\"$DIR/lock\"; TMUX=\"$RHOST_TMUX\"\n")
	b.WriteString("exec 9>\"$LOCK\" || { echo RHOST_ERR=nosession; exit 0; }\n")
	b.WriteString("flock -n 9 || { echo RHOST_ERR=locked; exit 0; }\n")
	b.WriteString("tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n")
	b.WriteString(shellOfFunc)
	b.WriteString("want=$(RHOST_SHELL_OF \"$DIR/meta.json\" 2>/dev/null); [ -n \"$want\" ] || want=" + shell.Quote(DefaultShell) + "\n")
	b.WriteString("TTY=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_tty}' 2>/dev/null)\n")
	b.WriteString("[ -n \"$TTY\" ] || { echo RHOST_ERR=notty; exit 0; }\n")
	b.WriteString("stty -echo < \"$TTY\" 2>/dev/null || { echo RHOST_ERR=notty; exit 0; }\n")
	b.WriteString(markerScanFunc)
	b.WriteString("start=$(wc -c < \"$LOG\" 2>/dev/null || echo 0); dpat=$(printf '\\033]133;D;'); dlen=${#dpat}\n")
	b.WriteString("tmux send-keys -t \"$TMUX:0.0\" C-c 2>/dev/null\n")
	p("SECONDS=0; ready=0; floor=$((start + 1)); scan=$floor; while [ \"$SECONDS\" -lt %d ]; do\n", timeoutSec)
	b.WriteString("  tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=sessiondied; exit 0; }\n")
	b.WriteString("  fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n")
	b.WriteString("  if [ \"$fg\" = \"$want\" ] && rh_window; then ready=1; break; fi\n")
	b.WriteString("  sleep 0.1\n")
	b.WriteString("done\n")
	b.WriteString("fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n")
	b.WriteString("if [ -z \"$fg\" ]; then echo RHOST_ERR=unknownfg; exit 0; fi\n")
	b.WriteString("if [ \"$fg\" != \"$want\" ]; then echo \"RHOST_FG=$fg\"; echo RHOST_ERR=busy; exit 0; fi\n")
	b.WriteString("[ \"$ready\" = 1 ] || { echo RHOST_ERR=notready; exit 0; }\n")
	b.WriteString("echo RHOST_OK=recovered\n")
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
		case line == "":
		default:
			out.Err = "protocol"
			return out
		}
	}
	if expectedToken == "" || len(tokens) != 1 || tokens[0] != expectedToken || len(exits) != 1 || len(outputs) != 1 {
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
