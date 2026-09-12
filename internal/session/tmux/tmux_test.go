package tmux

import (
	"encoding/base64"
	"strings"
	"testing"
	"time"
)

func TestParseExec(t *testing.T) {
	payload := base64.StdEncoding.EncodeToString([]byte("hello\nworld\n"))

	out := ParseExec("RHOST_EXIT=3\n" + payload + "\n")
	if out.Err != "" {
		t.Fatalf("unexpected err %q", out.Err)
	}
	if out.ExitCode != 3 {
		t.Errorf("exit code = %d, want 3", out.ExitCode)
	}
	if out.Output != "hello\nworld\n" {
		t.Errorf("output = %q", out.Output)
	}

	tout := ParseExec("RHOST_ERR=timeout\n")
	if tout.Err != "timeout" {
		t.Errorf("err = %q, want timeout", tout.Err)
	}

	// The refusal of a pane owned by another program names that program, because
	// the caller's next move depends on what it is.
	busy := ParseExec("RHOST_FG=python3\nRHOST_ERR=busy\n")
	if busy.Err != "busy" || busy.Foreground != "python3" {
		t.Errorf("busy refusal = %+v, want err=busy foreground=python3", busy)
	}

	// A command that outlived its deadline is interrupted, and the helper answers
	// with whether the pane actually came back. "timed out" and "timed out but the
	// session is usable" are different instructions to a caller, so the two states
	// must survive parsing.
	if out := ParseExec("RHOST_RECOVERED=1\nRHOST_ERR=timeout\n"); out.Err != "timeout" || !out.Recovered {
		t.Errorf("recovered timeout parsed as %+v", out)
	}
	if out := ParseExec("RHOST_RECOVERED=0\nRHOST_ERR=timeout\n"); out.Err != "timeout" || out.Recovered {
		t.Errorf("unrecovered timeout parsed as %+v", out)
	}
	// The marker is only ever printed next to a timeout; a clean command must not
	// pick up a recovered flag out of nowhere.
	if out := ParseExec("RHOST_EXIT=0\n" + payload + "\n"); out.Recovered {
		t.Errorf("successful command reports Recovered")
	}
}

func TestParseRead(t *testing.T) {
	payload := base64.StdEncoding.EncodeToString([]byte("abc"))
	out := ParseRead("RHOST_FROM=10\nRHOST_NEXT=13\nRHOST_SIZE=20\n" + payload + "\n")
	if out.From != 10 || out.Next != 13 || out.Size != 20 {
		t.Errorf("cursors = (%d,%d,%d)", out.From, out.Next, out.Size)
	}
	if string(out.Data) != "abc" {
		t.Errorf("data = %q", out.Data)
	}
}

func TestParseList(t *testing.T) {
	meta := `{"schema_version":1,"id":"s_1","name":"dev","tmux_session":"rhost_s_s_1","created_at":"2026-09-10T00:00:00Z","shell":"bash"}`
	line := "RHOST_META\ts_1\tyes\t" + base64.StdEncoding.EncodeToString([]byte(meta))
	entries := ParseList("noise\n" + line + "\n")
	if len(entries) != 1 {
		t.Fatalf("entries = %d, want 1", len(entries))
	}
	e := entries[0]
	if e.ID != "s_1" || !e.Alive || e.Meta.Name != "dev" || e.Meta.TmuxSession != "rhost_s_s_1" {
		t.Errorf("unexpected entry: %+v (meta %+v)", e, e.Meta)
	}
}

func TestCreateScriptProtocol(t *testing.T) {
	meta := NewMeta("s_abc", "dev", "/tmp", "bash")
	s := CreateScript(meta, "bash --noprofile --norc -i")
	for _, want := range []string{
		"tmux new-session -d -s 'rhost_s_s_abc'",
		"pipe-pane",
		"pane_current_command",
		"base64 -d | tmux load-buffer",
		"$DIR/ready",
		"meta.json",
		"RHOST_OK=created",
		"RHOST_ERR=notmux",
		"RHOST_ERR=nameinuse",
		"RHOST_ERR=newfailed",
		"RHOST_NAME_OF",
	} {
		if !contains(s, want) {
			t.Errorf("CreateScript missing %q", want)
		}
	}
}

// TestCreateScriptNotReadyCleansUp pins the failure-path fix: a session that
// never becomes ready must kill its freshly created tmux session and remove its
// state dir, or it becomes an orphan invisible to `session list`.
func TestCreateScriptNotReadyCleansUp(t *testing.T) {
	meta := NewMeta("s_abc", "dev", "", "bash")
	s := CreateScript(meta, "bash --noprofile --norc -i")
	for _, want := range []string{
		"tmux kill-session -t 'rhost_s_s_abc'",
		"rm -rf \"$DIR\"",
		"RHOST_ERR=notready",
	} {
		if !contains(s, want) {
			t.Errorf("CreateScript notready path missing %q", want)
		}
	}
}

func TestCreateScriptSerializesNamesAndCleansInterruptedBootstrap(t *testing.T) {
	s := CreateScript(NewMeta("s_abc", "dev", "", "bash"), "bash --noprofile --norc -i")
	lock := indexOf(s, `flock -w 30 8`)
	scan := indexOf(s, `for d in "$BASE"/sessions/*/`)
	create := indexOf(s, `tmux new-session`)
	arm := indexOf(s, `created=yes`)
	meta := indexOf(s, `> "$DIR/meta.json"`)
	disarm := strings.LastIndex(s, `created=no`)
	for _, want := range []string{
		`command -v flock`, `exec 8> "$BASE/sessions/.create.lock"`,
		`trap cleanup EXIT`, `trap 'exit 1' HUP INT TERM`,
		`tmux kill-session`, `rm -rf "$DIR"`,
	} {
		if !contains(s, want) {
			t.Errorf("CreateScript lifecycle guard missing %q:\n%s", want, s)
		}
	}
	if lock < 0 || scan < 0 || lock > scan {
		t.Errorf("duplicate-name scan is not under the global lock: lock=%d scan=%d", lock, scan)
	}
	if create < 0 || arm < create || meta < arm || disarm < meta {
		t.Errorf("cleanup lifecycle is not create -> arm -> metadata -> disarm: %d %d %d %d", create, arm, meta, disarm)
	}
}

func TestIntegrationScriptReadyPath(t *testing.T) {
	s := integrationScript("s_abc")
	for _, want := range []string{
		"stty -echo",
		"133;D;",
		"PROMPT_COMMAND=__rh_done",
		"/sessions/s_abc/ready",
	} {
		if !contains(s, want) {
			t.Errorf("integrationScript missing %q", want)
		}
	}
}

func TestExecScriptProtocol(t *testing.T) {
	s := ExecScript("dev", "echo hi", 5*time.Second)
	for _, want := range []string{
		"flock",
		"RHOST_ERR=noflock",
		"RHOST_RESOLVE",
		"133;D;",
		"sessiondied",
		"RHOST_EXIT=",
		"stty -echo",
		"tmux paste-buffer",
		"grep -aqF",
		"SECONDS=0",
	} {
		if !contains(s, want) {
			t.Errorf("ExecScript missing %q", want)
		}
	}
	// After the deadline the helper must interrupt *and then prove* the shell is
	// back, in that order, before it reports the timeout: an unverified Ctrl-C is
	// how a session ends up with two commands racing in one pane.
	interrupt := indexOf(s, `send-keys -t "$TMUX:0.0" C-c`)
	probe := indexOf(s, `recovered=1`)
	report := indexOf(s, `echo RHOST_ERR=timeout`)
	if interrupt < 0 || probe < 0 || report < 0 {
		t.Fatalf("ExecScript has no interrupt/probe/report sequence: %d %d %d", interrupt, probe, report)
	}
	if !(interrupt < probe && probe < report) {
		t.Errorf("ExecScript reports a timeout without proving recovery first: %d %d %d",
			interrupt, probe, report)
	}
	if !contains(s, "RHOST_RECOVERED=$recovered") {
		t.Errorf("ExecScript must carry the recovery answer out to the CLI")
	}
}

// TestExecScriptScansEveryByte is the regression test for a real failure:
// `session exec -- "bash -c 'exit 4'"` timed out on a live host although the
// completion marker had been written. The scan advanced its offset to a size it
// had measured but not read, so a marker that arrived in between sat in bytes the
// loop never looked at again — and a finished command became a spurious timeout.
//
// Two properties make the scan correct, and both are asserted here because the
// race itself is not reproducible on demand:
//
//   - each window re-reads the last (marker length - 1) bytes, so no 9-byte marker
//     can straddle two polls;
//   - `scan` only ever advances to a size that has actually been read, and the
//     window never reaches before `floor`, so an old command's marker cannot be
//     mistaken for this one's.
func TestExecScriptScansEveryByte(t *testing.T) {
	s := ExecScript("dev", "echo hi", 5*time.Second)
	for _, want := range []string{
		"rh_window() {",
		"from=$((scan - dlen + 1))",
		`[ "$from" -lt "$floor" ] && from=$floor`,
		"head -c $((cur - from + 1))",
		"scan=$cur",
		"floor=$((start + 1)); scan=$floor",
		"floor=$((pre + 1)); scan=$floor",
	} {
		if !contains(s, want) {
			t.Errorf("ExecScript scan missing %q\n---\n%s", want, s)
		}
	}
	// The old shape: measure, then jump the offset past what was read.
	if contains(s, "over=$((cur - (dlen - 1)))") {
		t.Errorf("ExecScript still advances the scan past unread bytes")
	}
	// Exit-code extraction must take the digits after the marker it located, not
	// the last `;D;` in a 24-byte window that may hold two markers.
	for _, want := range []string{"tmp=${seg#\"$dpat\"}", "code=${tmp%%[!0-9]*}"} {
		if !contains(s, want) {
			t.Errorf("ExecScript exit-code extraction missing %q", want)
		}
	}
	if contains(s, "sed -n 's/.*;D;") {
		t.Errorf("ExecScript still extracts the exit code with a greedy sed")
	}
}

// TestListAndSendPreflight checks the remaining tmux-backed scripts fail fast
// with a stable code on a host without tmux, instead of misreporting liveness
// or a missing session.
func TestListAndSendPreflight(t *testing.T) {
	if s := ListScript(); !contains(s, "RHOST_ERR=notmux") {
		t.Errorf("ListScript missing tmux preflight")
	}
	if s := SendScript("dev", "data", "x"); !contains(s, "RHOST_ERR=notmux") {
		t.Errorf("SendScript missing tmux preflight")
	}
	if s := EchoScript("dev", true); !contains(s, "RHOST_ERR=notmux") {
		t.Errorf("EchoScript missing tmux preflight")
	}
}

// TestExecScriptRefusesWhenPaneIsBusy is the §16 guard: a command is pasted into
// the terminal, so the terminal must belong to the managed shell. The check has
// to come *before* the first byte is sent — the stty probe is itself input — and
// it has to fail with its own code rather than degrade into a timeout.
func TestExecScriptRefusesWhenPaneIsBusy(t *testing.T) {
	s := ExecScript("dev", "echo hi", 5*time.Second)
	for _, want := range []string{
		`want=$(RHOST_SHELL_OF "$DIR/meta.json"`,
		`#{pane_current_command}`,
		`echo "RHOST_FG=$fg"`,
		"RHOST_ERR=busy",
		"RHOST_ERR=unknownfg",
		// The metadata carries the shell the session was created with.
		`"shell"`,
	} {
		if !contains(s, want) {
			t.Errorf("ExecScript missing the foreground check (%q):\n%s", want, s)
		}
	}
	gate := indexOf(s, "RHOST_ERR=busy")
	probe := indexOf(s, `send-keys -t "$TMUX:0.0" 'stty -echo`)
	paste := indexOf(s, "tmux paste-buffer -b rhost_cmd")
	if gate < 0 || probe < 0 || paste < 0 {
		t.Fatalf("ExecScript has no foreground-check/probe/paste sequence: %d %d %d", gate, probe, paste)
	}
	if !(gate < probe && gate < paste) {
		t.Errorf("ExecScript must check the foreground before typing anything: %d %d %d", gate, probe, paste)
	}
	// The lock is taken before the check, so two writers cannot both inspect the
	// pane and then both paste.
	lock := indexOf(s, "flock -n 9")
	if lock < 0 || lock > gate {
		t.Errorf("ExecScript must hold the writer lock before checking the pane: %d %d", lock, gate)
	}
}

// TestEchoScriptSetsThePaneTTY pins the fix for the attach path: echo is a
// property of the pane's pty, so it is set on that pty. Typing `stty …` into the
// pane would deliver the keystrokes to whatever owns the foreground — the bug
// this replaced — and would fail to change the terminal at all when the shell is
// not the foreground process.
func TestEchoScriptSetsThePaneTTY(t *testing.T) {
	on := EchoScript("dev", true)
	off := EchoScript("dev", false)
	for _, s := range []string{on, off} {
		if contains(s, "send-keys") {
			t.Errorf("EchoScript must not type into the pane:\n%s", s)
		}
		if !contains(s, `#{pane_tty}`) || !contains(s, ` < "$TTY"`) {
			t.Errorf("EchoScript must set the flag on the pane's tty:\n%s", s)
		}
		if !contains(s, "RHOST_ERR=notty") {
			t.Errorf("EchoScript must report a missing tty with its own code:\n%s", s)
		}
	}
	if !contains(on, `stty echo < "$TTY"`) {
		t.Errorf("echo on must set the flag:\n%s", on)
	}
	if !contains(off, `stty -echo < "$TTY"`) || contains(off, `stty echo < "$TTY"`) {
		t.Errorf("echo off must clear the flag and only the flag:\n%s", off)
	}
}

func TestReadScriptCursor(t *testing.T) {
	s := ReadScript("dev", 42, 1024)
	for _, want := range []string{"since=42", "RHOST_FROM=", "RHOST_NEXT=", "RHOST_SIZE=", "head -c 1024"} {
		if !contains(s, want) {
			t.Errorf("ReadScript missing %q", want)
		}
	}
}

func contains(haystack, needle string) bool {
	return len(needle) == 0 || (len(haystack) >= len(needle) && indexOf(haystack, needle) >= 0)
}

func indexOf(s, sub string) int {
	for i := 0; i+len(sub) <= len(s); i++ {
		if s[i:i+len(sub)] == sub {
			return i
		}
	}
	return -1
}
