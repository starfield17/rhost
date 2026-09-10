package detached

import (
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/shell"
)

// TestStateMachine pins the documented state derivation (docs/ARCHITECTURE.md
// §23): an exit-code file is authoritative, a live pid is running, a dead pid
// with no exit code is stale (never success), and a stopped marker moves any
// final outcome to "stopped".

func TestStateFromFacts(t *testing.T) {
	cases := []struct {
		name string
		f    Facts
		want State
	}{{
		name: "exited-zero", f: Facts{ExitCode: 0}, want: StateExited,
	}, {
		name: "failed-nonzero", f: Facts{ExitCode: 3}, want: StateFailed,
	}, {
		name: "stopped-request-then-exit-0", f: Facts{ExitCode: 0, Stopped: true}, want: StateStopped,
	}, {
		name: "stopped-request-then-signal", f: Facts{ExitCode: 143, Stopped: true}, want: StateStopped,
	}, {
		name: "running", f: Facts{PID: 41, Alive: true, ExitCode: -1}, want: StateRunning,
	}, {
		name: "starting-no-pid-yet", f: Facts{PID: 0, Alive: false, ExitCode: -1}, want: StateStarting,
	}, {
		name: "stale-died-no-exit-code", f: Facts{PID: 41, Alive: false, ExitCode: -1}, want: StateStale,
	}, {
		name: "stopped-request-died-no-exit-code", f: Facts{PID: 41, Alive: false, ExitCode: -1, Stopped: true}, want: StateStopped,
	}, {
		name: "unknown-format-empty", f: Facts{ExitCode: -1}, want: StateStarting,
	}}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := StateFromFacts(tc.f); got != tc.want {
				t.Errorf("StateFromFacts(%+v) = %s, want %s", tc.f, got, tc.want)
			}
		})
	}
}

// The start response never claims a state it cannot know: with no exit code and
// a pid, the only honest answers are running or stale, and the state machine
// picks exactly those.
func TestStartFactsDerivation(t *testing.T) {
	f := ParseStartOrDie(t, "RHOST_JOB=j_abc\nRHOST_PID=123\nRHOST_ALIVE=yes\nRHOST_EXIT=-1\nRHOST_STOPPED=no\nRHOST_FINISHED_AT=\n")
	if got := StateFromFacts(f); got != StateRunning {
		t.Errorf("live pid should be running, got %s", got)
	}
}

func TestParseStartHelperError(t *testing.T) {
	_, err := ParseStart("RHOST_ERR=nojob\n")
	if err != NotFound {
		t.Fatalf("ParseStart = %q, want nojob", err)
	}
}

func TestCommandScriptRecordsPidAndTrap(t *testing.T) {
	s := CommandScript("j_x", "/work", map[string]string{"A": "1"}, "echo hi\nexit 4\n")
	for _, want := range []string{
		`printf '%s\n' "$$" > "$D/pid"`,
		`> "$D/exit_code"`,
		"trap '",
		"export A='1'\n",
		"cd -- '/work' 2>/dev/null ||",
		"(\necho hi\nexit 4\n)\n",
	} {
		if !strings.Contains(s, want) {
			t.Errorf("CommandScript missing %q\n---\n%s", want, s)
		}
	}
	// A stopped job must record its signal status, never the initial rc of 0:
	// without these traps bash runs the EXIT trap after a SIGTERM death with rc
	// untouched, and `job status` would show exit_code 0 for a job that was
	// terminated. Live-tested; pinned here.
	for _, want := range []string{
		"trap 'rc=143; exit 143' TERM",
		"trap 'rc=130; exit 130' INT",
		"trap 'rc=129; exit 129' HUP",
		"trap 'rc=131; exit 131' QUIT",
	} {
		if !strings.Contains(s, want) {
			t.Errorf("CommandScript must trap %q\n---\n%s", want, s)
		}
	}
	// SIGKILL is not, and cannot be, trapped: `job kill` therefore leaves no exit
	// code, and the state machine reports `stopped` from the marker alone.
	if strings.Contains(s, "KILL") {
		t.Errorf("CommandScript must not trap KILL: %s", s)
	}
}

func TestStartScriptPreflightsAndDetaches(t *testing.T) {
	meta := NewMeta("j_x", "train", "/work", "python train.py")
	script := StartScript(meta, "#!/usr/bin/env bash\n")
	for _, want := range []string{
		"RHOST_ERR=nobash",
		"RHOST_ERR=nosetsid",
		"RHOST_ERR=nonohup",
		"setsid nohup bash \"$DIR/command.sh\"",
		`>"$DIR/stdout.log"`,
		`2>"$DIR/stderr.log"`,
		"sleep 0.1",
	} {
		if !strings.Contains(script, want) {
			t.Errorf("StartScript missing %q\n---\n%s", want, script)
		}
	}
}

func TestLogsScriptCursorArithmetic(t *testing.T) {
	s := LogsScript("j_x", "stdout", 4096, 0)
	for _, want := range []string{
		"RHOST_ERR=nojob",
		`[ "$since" -gt "$size" ] && since=$size`,
		"head -c 262144",
		`"$((since + inc))"`,
		"base64 -w0",
	} {
		if !strings.Contains(s, want) {
			t.Errorf("LogsScript missing %q\n---\n%s", want, s)
		}
	}
}

// Every helper that can create a file must do so under a private umask
// (docs/ARCHITECTURE.md §21). The `stopped` marker once landed 0664 because only
// the start path set a umask; this check covers every script, including any
// future one that starts from basePreamble.
func TestScriptsKeepRemoteStatePrivate(t *testing.T) {
	scripts := map[string]string{
		"start":  StartScript(NewMeta("j_x", "j_x", "", "true"), "#!/usr/bin/env bash\n"),
		"status": StatusScript("j_x"),
		"logs":   LogsScript("j_x", "stdout", 0, 0),
		"signal": SignalScript("j_x", false),
		"list":   ListScript(),
		"cmd":    CommandScript("j_x", "", nil, "true"),
	}
	for name, s := range scripts {
		if !strings.Contains(s, "umask 077") {
			t.Errorf("%s script must set umask 077:\n%s", name, s)
		}
	}
}

// A job handle is data, never code. Each of these scripts once interpolated the
// caller's argument straight into `DIR="$BASE/jobs/<arg>"`, so an argument
// containing a double quote closed the string and ran whatever followed it on the
// remote host (AGENTS.md §5: OpenSSH owns auth; rhost adds no new way to execute
// things). Every path built from a caller-supplied ref must therefore appear in
// its single-quoted form, and the raw text must never sit inside an open quote.
func TestScriptsQuoteTheJobHandle(t *testing.T) {
	const hostile = `x"; touch /tmp/pwned; #`
	quoted := shell.Quote(hostile)

	scripts := map[string]string{
		"start":  StartScript(NewMeta(hostile, hostile, "", "true"), "#!/usr/bin/env bash\n"),
		"status": StatusScript(hostile),
		"logs":   LogsScript(hostile, "stdout", 0, 0),
		"signal": SignalScript(hostile, false),
		"cmd":    CommandScript(hostile, hostile, map[string]string{"A": hostile}, hostile),
	}
	for name, s := range scripts {
		if !strings.Contains(s, quoted) {
			t.Errorf("%s script must quote the handle (%s):\n%s", name, quoted, s)
		}
		if strings.Contains(s, "jobs/"+hostile) {
			t.Errorf("%s script interpolates the raw handle into a path:\n%s", name, s)
		}
	}
	// The directory assignment must be the concatenation of a quoted prefix and a
	// quoted ref, which is what makes the quoting effective.
	if s := StatusScript(hostile); !strings.Contains(s, `DIR="$BASE/jobs/"`+quoted+"\n") {
		t.Errorf("status script must build DIR by concatenation:\n%s", s)
	}
	// A stream name goes through the same rule even though the app layer
	// whitelists it: `".log"` stays a literal suffix.
	if s := LogsScript("j_x", "../../evil", 0, 0); !strings.Contains(s, `LOG="$DIR/"'../../evil'".log"`) {
		t.Errorf("logs script must quote the stream name:\n%s", s)
	}
	// --cwd and --env values are quoted too, and never concatenated into a path.
	if s := CommandScript("j_x", hostile, nil, "true"); !strings.Contains(s, "cd -- "+quoted) {
		t.Errorf("cwd must be quoted:\n%s", s)
	}
}

// The list row and its parser are one contract with two ends; pin the shape, so
// a column added to the script cannot silently shift meta into the pid field.
func TestListScriptRowShape(t *testing.T) {
	s := ListScript()
	if !strings.Contains(s, `"$id" "$pid" "$alive" "$ec" "$stopped" "$fa" "$meta"`) {
		t.Errorf("ListScript row must be id, pid, alive, exit, stopped, finished_at, meta:\n%s", s)
	}
	if !strings.Contains(s, `[ -d "$BASE/jobs" ] || exit 0`) {
		t.Errorf("ListScript must tolerate a host with no jobs yet:\n%s", s)
	}
}

func TestSignalScriptGroupKill(t *testing.T) {
	s := SignalScript("j_x", false)
	if !strings.Contains(s, `kill -TERM -"$pid" 2>/dev/null || true`) {
		t.Errorf("SignalScript must signal the process group: %s", s)
	}
	// The stopped marker must be written only while the process is alive, so a
	// stop request never relabels an already-recorded result.
	if !strings.Contains(s, "if [ \"$alive\" = yes ]; then\n: > \"$DIR/stopped\"") {
		t.Errorf("SignalScript must gate the stopped marker on liveness: %s", s)
	}
	// `kill` selects SIGKILL and nothing else may.
	if !strings.Contains(SignalScript("j_x", true), "kill -KILL -") {
		t.Errorf("kill=true must send SIGKILL: %s", SignalScript("j_x", true))
	}
	// A stop against a missing job must be a distinct error, not a silent no-op.
	if !strings.Contains(s, "RHOST_ERR=nojob") {
		t.Errorf("SignalScript must distinguish unknown jobs: %s", s)
	}
	// The reported facts must be re-measured after the group has had a bounded
	// grace period to die, otherwise `job stop` answers "running" for a job it
	// has already killed and the caller cannot tell the two apart.
	if !strings.Contains(s, "while [ \"$i\" -lt 25 ]") {
		t.Errorf("SignalScript must wait for the signal to take effect: %s", s)
	}
	if n := strings.Count(s, "ps -o stat= -p"); n < 2 {
		t.Errorf("SignalScript must measure facts again after signalling, measured %d times: %s", n, s)
	}
}

// The grace period is bounded so a job that ignores SIGTERM cannot hang the
// CLI: the helper always returns, and reports the job as still running.
func TestSignalGracePeriodIsBounded(t *testing.T) {
	if signalGraceTicks <= 0 || signalGraceTicks > 100 {
		t.Errorf("unbounded signal grace period: %d ticks", signalGraceTicks)
	}
}

func TestParseLogsRoundTrip(t *testing.T) {
	payload := "hello\n\x00world"
	b64 := base64.StdEncoding.EncodeToString([]byte(payload))
	stdout := "RHOST_SINCE=5\nRHOST_SIZE=12\nRHOST_NEXT=12\n" + b64 + "\n"
	out, err := ParseLogs(stdout)
	if err != "" {
		t.Fatalf("ParseLogs: %s", err)
	}
	if out.From != 5 || out.Size != 12 || out.Next != 12 {
		t.Errorf("cursors wrong: %+v", out)
	}
	if string(out.Data) != payload {
		t.Errorf("data = %q, want %q", out.Data, payload)
	}
}

func TestParseStatusCarriesMeta(t *testing.T) {
	meta := NewMeta("j_1", "train", "/work", "python train.py")
	mb := base64.StdEncoding.EncodeToString(mustMarshal(t, meta))
	stdout := "RHOST_JOB=j_1\nRHOST_PID=7\nRHOST_ALIVE=no\nRHOST_EXIT=3\nRHOST_STOPPED=no\nRHOST_FINISHED_AT=2026-09-10T12:00:00Z\nRHOST_META=" + mb + "\n"
	f, err := ParseStatus(stdout)
	if err != "" {
		t.Fatalf("ParseStatus: %s", err)
	}
	if f.ID != "j_1" || f.PID != 7 || f.Alive || f.ExitCode != 3 || f.FinishedAt == "" {
		t.Errorf("facts wrong: %+v", f)
	}
	if f.Meta == nil || f.Meta.Command != "python train.py" || f.Meta.Backend != Backend {
		t.Errorf("meta wrong: %+v", f.Meta)
	}
}

func TestParseList(t *testing.T) {
	meta := NewMeta("j_2", "", "", "echo x")
	mb := base64.StdEncoding.EncodeToString(mustMarshal(t, meta))
	stdout := "RHOST_META\tj_2\t4412\tyes\t-1\tno\t\t" + mb + "\n"
	list := ParseList(stdout)
	if len(list) != 1 {
		t.Fatalf("ParseList = %d entries, want 1", len(list))
	}
	f := list[0]
	if f.ID != "j_2" || f.PID != 4412 || !f.Alive || f.ExitCode != -1 || f.Meta == nil {
		t.Errorf("list entry wrong: %+v", f)
	}
}

// A list row with no pid must not become pid 0 by accident of parsing: an empty
// field means "never had one", which is a different fact from 4412.
func TestParseListRowWithoutPid(t *testing.T) {
	list := ParseList("RHOST_META\tj_3\t\tno\t0\tno\t2026-09-10T12:00:00Z\t\n")
	if len(list) != 1 {
		t.Fatalf("want 1 entry, got %d", len(list))
	}
	f := list[0]
	if f.PID != 0 || f.ExitCode != 0 || f.Meta != nil {
		t.Errorf("entry wrong: %+v", f)
	}
	if got := StateFromFacts(f); got != StateExited {
		t.Errorf("state = %s, want exited", got)
	}
}

// Unreadable exit-code content must stay "no code recorded": a parse failure
// that fell through to 0 would be a false report of success.
func TestParseStatusRejectsGarbageExitCode(t *testing.T) {
	f, err := ParseStatus("RHOST_JOB=j_9\nRHOST_PID=7\nRHOST_ALIVE=no\nRHOST_EXIT=\nRHOST_STOPPED=no\nRHOST_FINISHED_AT=\n")
	if err != "" {
		t.Fatalf("ParseStatus: %s", err)
	}
	if f.ExitCode != -1 {
		t.Errorf("ExitCode = %d, want -1", f.ExitCode)
	}
	if got := StateFromFacts(f); got != StateStale {
		t.Errorf("state = %s, want stale (never exited)", got)
	}
}

// helpers

func ParseStartOrDie(t *testing.T, stdout string) Facts {
	t.Helper()
	f, err := ParseStart(stdout)
	if err != "" {
		t.Fatalf("ParseStart: %s", err)
	}
	return f
}

func mustMarshal(t *testing.T, v interface{}) []byte {
	t.Helper()
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return b
}
