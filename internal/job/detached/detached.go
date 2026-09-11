// Package detached implements rhost's v0.1 job backend: a long-running remote
// command launched with `setsid nohup bash` and owned entirely by remote files,
// so it survives the CLI process and SSH disconnects (docs/ARCHITECTURE.md §22).
//
// On the remote host each job lives under
//
//	$RHOST_REMOTE_STATE/jobs/<id>/
//	    meta.json     discovery metadata (authoritative)
//	    command.sh    the generated wrapper that runs the user's command
//	    stdout.log    the job's stdout, appended by the shell
//	    stderr.log    the job's stderr
//	    pid           the detached process's pid (== its pgid: it is a session
//	                  leader by construction)
//	    pgid          same value, kept for explicitness
//	    exit_code     written atomically by an EXIT trap when the job finishes
//	    finished_at   RFC3339 completion time, written with exit_code
//	    stopped       marker written by `job stop`/`job kill` before signalling
//
// The job process writes its own pid file before running the command; it is a
// session leader, so signalling the process group kills the whole tree.
package detached

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/shell"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// Backend is the backend identifier recorded in job metadata.
const Backend = "detached"

// basePreamble defines the remote state root used by every script and keeps
// anything a helper creates user-private (docs/ARCHITECTURE.md §21): `job stop`
// writes the stopped marker, and without this umask that file lands 0664 from
// the login session's default. It mirrors the expression used by sessions so one
// override (RHOST_REMOTE_STATE) moves both trees.
const basePreamble = "BASE=\"${RHOST_REMOTE_STATE:-" + openssh.DefaultRemoteStateDir + "}\"\numask 077\n"

// Meta is the discovery metadata for a managed job (docs/ARCHITECTURE.md §21).
type Meta struct {
	SchemaVersion int    `json:"schema_version"`
	ID            string `json:"id"`
	Name          string `json:"name,omitempty"`
	Cwd           string `json:"cwd,omitempty"`
	Command       string `json:"command"`
	StartedAt     string `json:"started_at"`
	Backend       string `json:"backend"`
}

// NewMeta builds metadata for a new job.
func NewMeta(id, name, cwd, command string) Meta {
	return Meta{
		SchemaVersion: 1,
		ID:            id,
		Name:          name,
		Cwd:           cwd,
		Command:       command,
		StartedAt:     time.Now().UTC().Format(time.RFC3339),
		Backend:       Backend,
	}
}

// State is the derived job lifecycle state (docs/ARCHITECTURE.md §23). The
// machine is implemented in StateFromFacts and pinned by table tests.
type State string

const (
	StateStarting State = "starting" // registered but no pid yet
	StateRunning  State = "running"  // pid alive
	StateExited   State = "exited"   // exit_code == 0 recorded
	StateFailed   State = "failed"   // exit_code != 0 recorded
	StateStopped  State = "stopped"  // stop/kill requested and final condition held
	StateUnknown  State = "unknown"  // cannot be determined (never used for a dir with meta)
	StateStale    State = "stale"    // died without recording an exit code
)

// Facts is the raw, observed state of one job: what the remote host can tell
// us without any interpretation. The state machine in StateFromFacts is the
// only place facts become a State.
type Facts struct {
	ID         string
	PID        int
	Alive      bool
	ExitCode   int // -1 when no exit_code file exists
	Stopped    bool
	FinishedAt string
	Meta       *Meta // nil when the script did not carry metadata
}

// StateFromFacts derives a job state from observed facts.
//
// The machine, deliberately: an exit-code file is authoritative — the job
// ended, and ended *our way* (the EXIT trap wrote it), so we report the result.
// A live pid is running. A dead pid with no exit-code file did not end our way
// (SIGKILL, host reboot): that is stale, never success. A job with neither pid
// nor exit code is still starting. A stopped marker moves any terminal outcome
// to "stopped": the requested final condition holds (docs/ARCHITECTURE.md §25).
func StateFromFacts(f Facts) State {
	switch {
	case f.ExitCode >= 0:
		if f.Stopped {
			return StateStopped
		}
		if f.ExitCode == 0 {
			return StateExited
		}
		return StateFailed
	case f.PID <= 0:
		return StateStarting
	case f.Alive:
		return StateRunning
	case f.Stopped:
		return StateStopped
	default:
		return StateStale
	}
}

// dirAssign binds DIR to one job's state directory, and is the only way a
// caller-supplied ref reaches a remote path. The ref is shell-quoted rather than
// interpolated: a job handle is *data*, and unquoted data inside a double-quoted
// assignment can close the string and run whatever follows it (AGENTS.md §5,
// docs/ARCHITECTURE.md §35). The app layer also refuses shapes that are neither
// an id nor a name, so this is the second of two independent guards.
func dirAssign(ref string) string {
	return "DIR=\"$BASE/jobs/\"" + shell.Quote(ref) + "\n"
}

// logAssign binds LOG to one stream of a job. `stream` is validated by the app
// layer to "stdout" or "stderr"; quoting it keeps this file correct even when a
// caller forgets, and the `.log` suffix stays outside the quoted part so it
// remains a literal rather than text a caller chooses.
func logAssign(stream string) string {
	return "LOG=\"$DIR/\"" + shell.Quote(stream) + `".log"` + "\n"
}

// factsBlock is the shared shell snippet every script that observes a job uses
// to measure and print RHOST_* fact lines. includeMeta adds the base64 meta
// (status needs it; stop/kill do not; list measures into variables and emits
// its own tabbed lines).
func factsBlock(id string, includeMeta bool) string {
	vars := factsVars()
	if includeMeta {
		vars += "meta=$(base64 -w0 < \"$DIR/meta.json\" 2>/dev/null || echo \"\")\n"
	}
	line := "printf 'RHOST_JOB=%s\\nRHOST_PID=%s\\nRHOST_ALIVE=%s\\nRHOST_EXIT=%s\\nRHOST_STOPPED=%s\\nRHOST_FINISHED_AT=%s\\n' " +
		shell.Quote(id) + " \"$pid\" \"$alive\" \"$ec\" \"$stopped\" \"$fa\"\n"
	if includeMeta {
		line = "printf 'RHOST_JOB=%s\\nRHOST_PID=%s\\nRHOST_ALIVE=%s\\nRHOST_EXIT=%s\\nRHOST_STOPPED=%s\\nRHOST_FINISHED_AT=%s\\nRHOST_META=%s\\n' " +
			shell.Quote(id) + " \"$pid\" \"$alive\" \"$ec\" \"$stopped\" \"$fa\" \"$meta\"\n"
	}
	return vars + line
}

// factsVars measures the raw facts into shell variables. Every script observes
// the same facts the same way; only the output shape differs per script.
func factsVars() string {
	return `pid=$(cat $DIR/pid 2>/dev/null || echo 0)
[ "$pid" = 0 ] && pid=""
` +
		// ps(1) rather than kill -0: a zombie briefly keeps its entry and kill -0
		// succeeds on it, which would report a reaped job as running.
		`st=$(ps -o stat= -p "$pid" 2>/dev/null); alive=no; case "$st" in *Z*) ;; *) [ -n "$st" ] && alive=yes ;; esac
` +
		`ec=-1; [ -f "$DIR/exit_code" ] && ec=$(cat "$DIR/exit_code" 2>/dev/null || echo -1)
` +
		`stopped=no; [ -f "$DIR/stopped" ] && stopped=yes
` +
		`fa=""; [ -f "$DIR/finished_at" ] && fa=$(cat "$DIR/finished_at" 2>/dev/null || true)
`
}

// Signal traps make a stopped job record the conventional 128+signum status
// instead of 0. Without them, bash runs the EXIT trap after a SIGTERM death with
// rc still at its initial 0, and the job would report a *successful* exit that
// never happened. Proven on a real remote bash 5.1: with these traps the
// recorded code is 143, without it the code is 0.
//
// SIGKILL is deliberately absent: it cannot be trapped, which is why `job kill`
// leaves no exit_code and the state machine reports `stopped` from the marker
// alone (never a fabricated status).
const signalTraps = `trap 'rc=130; exit 130' INT
trap 'rc=129; exit 129' HUP
trap 'rc=131; exit 131' QUIT
trap 'rc=143; exit 143' TERM
`

// CommandScript is the wrapper that becomes the detached job process itself.
//
// It writes its own pid (so the recorded pid is always the real session
// leader, never a short-lived intermediate), installs an EXIT trap that
// atomically records the exit code and finish time, then runs the user's
// command in a subshell so a bare `exit` inside it cannot skip the trap.
func CommandScript(id, cwd string, env map[string]string, command string) string {
	var b strings.Builder
	p := func(format string, a ...interface{}) { fmt.Fprintf(&b, format, a...) }

	b.WriteString("#!/usr/bin/env bash\n")
	b.WriteString("umask 077\n")
	// Double-quoted so $HOME / RHOST_REMOTE_STATE expand in this process at run
	// time: command.sh cannot inherit variables from the launcher (only exported
	// ones would carry over, and we export none).
	p("D=\"${RHOST_REMOTE_STATE:-%s}/jobs/\"%s\n", openssh.DefaultRemoteStateDir, shell.Quote(id))
	b.WriteString("printf '%s\\n' \"$$\" > \"$D/pid\"\n")
	b.WriteString("printf '%s\\n' \"$$\" > \"$D/pgid\"\n")
	b.WriteString("rc=0\n")
	b.WriteString("trap 'printf \"%s\\n\" \"$rc\" > \"$D/exit_code\"; date -u +%Y-%m-%dT%H:%M:%SZ > \"$D/finished_at\"; exit \"$rc\"' EXIT\n")
	b.WriteString(signalTraps)

	if cwd != "" {
		q := shell.PathQuote(cwd)
		// printf format is fixed; the (possibly hostile) path is an argument.
		p("cd -- %s 2>/dev/null || { rc=126; printf 'rhost: cannot change directory to %%s\\n' %s >&2; exit 0; }\n", q, q)
	}

	keys := make([]string, 0, len(env))
	for k := range env {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	for _, k := range keys {
		p("export %s=%s\n", k, shell.Quote(env[k]))
	}

	b.WriteString("(\n")
	b.WriteString(command)
	if !strings.HasSuffix(command, "\n") {
		b.WriteString("\n")
	}
	b.WriteString(")\n")
	b.WriteString("rc=$?\n")
	b.WriteString("exit 0\n")
	return b.String()
}

// StartScript launches a job and blocks until the detached process has
// registered its pid (or fails promptly, cleaning up so no orphan is left).
// It prints RHOST_ERR=... for preflight/start failures and RHOST_JOB/PID/ALIVE
// on success.
func StartScript(meta Meta, commandSH string) string {
	metaJSON, _ := json.Marshal(meta)

	var b strings.Builder

	b.WriteString(basePreamble)
	b.WriteString("command -v bash >/dev/null 2>&1 || { echo RHOST_ERR=nobash; exit 0; }\n")
	b.WriteString("command -v setsid >/dev/null 2>&1 || { echo RHOST_ERR=nosetsid; exit 0; }\n")
	b.WriteString("command -v nohup >/dev/null 2>&1 || { echo RHOST_ERR=nonohup; exit 0; }\n")
	b.WriteString(dirAssign(meta.ID))
	b.WriteString("mkdir -p \"$DIR\" && chmod 700 \"$DIR\" 2>/dev/null || { echo RHOST_ERR=mkdir; exit 0; }\n")
	b.WriteString("rm -f \"$DIR/pid\" \"$DIR/pgid\" \"$DIR/exit_code\" \"$DIR/finished_at\" \"$DIR/stopped\" \"$DIR/stdout.log\" \"$DIR/stderr.log\"\n")
	b.WriteString("printf '%s' '" + base64.StdEncoding.EncodeToString(metaJSON) + "' | base64 -d > \"$DIR/meta.json\"\n")
	b.WriteString("chmod 600 \"$DIR/meta.json\" 2>/dev/null\n")
	b.WriteString("printf '%s' '" + base64.StdEncoding.EncodeToString([]byte(commandSH)) + "' | base64 -d > \"$DIR/command.sh\"\n")
	b.WriteString("chmod 700 \"$DIR/command.sh\" 2>/dev/null || { rm -rf \"$DIR\"; echo RHOST_ERR=mkdir; exit 0; }\n")

	// The detached process is the only writer of stdout/stderr logs. Launch it
	// fully detached: its own session (setsid), SIGHUP-immune (nohup), no tty.
	b.WriteString("setsid nohup bash \"$DIR/command.sh\" >\"$DIR/stdout.log\" 2>\"$DIR/stderr.log\" < /dev/null &\n")
	b.WriteString("i=0; while [ \"$i\" -lt 50 ]; do [ -s \"$DIR/pid\" ] && break; sleep 0.1; i=$((i+1)); done\n")
	b.WriteString("if [ ! -s \"$DIR/pid\" ]; then rm -rf \"$DIR\"; echo RHOST_ERR=startfailed; exit 0; fi\n")
	b.WriteString(factsBlock(meta.ID, false))
	return b.String()
}

// StatusScript reports the observed facts of one job. The dir check makes a
// vanished job (unknown id) fail fast with RHOST_ERR=nojob.
func StatusScript(id string) string {
	return basePreamble +
		dirAssign(id) +
		"[ -f \"$DIR/meta.json\" ] || { echo RHOST_ERR=nojob; exit 0; }\n" +
		factsBlock(id, true)
}

// ListScript reports facts for every known job, one RHOST_META-tabbed line each
// carrying id, pid, liveness, exit code, stopped marker and the base64 meta.
//
// The pid is part of the row because `job list` must not claim pid 0 for a job
// that is demonstrably running (AGENTS.md §6: the JSON path has to be true, not
// just pretty).
func ListScript() string {
	return basePreamble + `[ -d "$BASE/jobs" ] || exit 0
for d in "$BASE"/jobs/*/; do
  [ -f "$d/meta.json" ] || continue
  id=$(basename "$d")
  DIR="$d"
` + factsVars() + `  meta=$(base64 -w0 < "$DIR/meta.json" 2>/dev/null || echo "")
  printf 'RHOST_META\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$id" "$pid" "$alive" "$ec" "$stopped" "$fa" "$meta"
done
`
}

// LogsScript returns the requested log stream from byte offset `since`,
// base64-encoded, with explicit size/next cursors so agents can poll
// incrementally (docs/ARCHITECTURE.md §24).
func LogsScript(id, stream string, since, maxBytes int) string {
	if maxBytes <= 0 {
		maxBytes = 256 * 1024
	}
	if since < 0 {
		since = 0
	}

	var b strings.Builder
	p := func(format string, a ...interface{}) { fmt.Fprintf(&b, format, a...) }
	b.WriteString(basePreamble)
	b.WriteString(dirAssign(id))
	b.WriteString("[ -f \"$DIR/meta.json\" ] || { echo RHOST_ERR=nojob; exit 0; }\n")
	b.WriteString(logAssign(stream))
	b.WriteString("size=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n")
	p("since=%d\n", since)
	b.WriteString("[ \"$since\" -gt \"$size\" ] && since=$size\n")
	p("inc=$(tail -c +$((since+1)) \"$LOG\" 2>/dev/null | head -c %d | wc -c)\n", maxBytes)
	b.WriteString("printf 'RHOST_SINCE=%s\\nRHOST_SIZE=%s\\nRHOST_NEXT=%s\\n' \"$since\" \"$size\" \"$((since + inc))\"\n")
	b.WriteString("tail -c +$((since+1)) \"$LOG\" 2>/dev/null | head -c \"$inc\" | base64 -w0\n")
	b.WriteString("printf '\\n'\n")
	return b.String()
}

// SignalScript stops (TERM) or kills (KILL) a running job's whole process group
// and emits the resulting facts so the caller can report the transition.
//
// `kill` is a bool rather than a signal-name string: nothing caller-supplied can
// then reach `kill -…`, which is the one place in this file where a literal has
// to be spliced into a command rather than an argument.
//
// The stopped marker is written only while the process is still alive: a job
// that already finished keeps its recorded exit code and state, because a stop
// request must not relabel a real result (docs/ARCHITECTURE.md §25). Both
// signals are idempotent — signalling a group that no longer exists is a no-op,
// and an already-final job satisfies the requested condition.
//
// After signalling, the script waits a bounded grace period for the group to
// actually disappear and re-measures. Without that wait, `job stop` reports the
// facts captured one instant before death — "running" — which is true but
// useless: the caller cannot tell a harvested job from one that ignored the
// signal. If the grace period expires with the process still alive, the script
// says so honestly (state stays running) instead of inventing "stopped".
func SignalScript(id string, kill bool) string {
	signal := "TERM"
	if kill {
		signal = "KILL"
	}
	return basePreamble +
		dirAssign(id) +
		"[ -f \"$DIR/meta.json\" ] || { echo RHOST_ERR=nojob; exit 0; }\n" +
		factsVars() +
		"if [ \"$alive\" = yes ]; then\n" +
		": > \"$DIR/stopped\"\n" +
		"kill -" + signal + " -\"$pid\" 2>/dev/null || true\n" +
		"i=0; while [ \"$i\" -lt " + strconv.Itoa(signalGraceTicks) + " ]; do [ \"$alive\" = no ] && break; sleep " + signalPollDelay + "\n" +
		factsVars() + "  i=$((i+1))\ndone\n" +
		"fi\n" +
		"printf 'RHOST_JOB=%s\\nRHOST_PID=%s\\nRHOST_ALIVE=%s\\nRHOST_EXIT=%s\\nRHOST_STOPPED=%s\\nRHOST_FINISHED_AT=%s\\n' " +
		shell.Quote(id) + " \"$pid\" \"$alive\" \"$ec\" \"$stopped\" \"$fa\"\n"
}

// How long `job stop`/`job kill` wait for the process group to die before
// reporting the facts as they then stand: 5s at 0.2s intervals. A command that
// ignores SIGTERM for longer than that is reported as still running; rhost does
// not escalate on its own, because `job kill` exists for exactly that case.
const (
	signalGraceTicks = 25
	signalPollDelay  = "0.2"
)
