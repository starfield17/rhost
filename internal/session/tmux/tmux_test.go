package tmux

import (
	"encoding/base64"
	"strings"
	"testing"
	"time"
)

func TestParseExec(t *testing.T) {
	const token = "0123456789abcdef0123456789abcdef"
	payload := base64.StdEncoding.EncodeToString([]byte("hello\nworld\n"))

	out := ParseExec("RHOST_TOKEN="+token+"\nRHOST_EXIT=3\nRHOST_OUTPUT="+payload+"\n", token)
	if out.Err != "" {
		t.Fatalf("unexpected err %q", out.Err)
	}
	if out.ExitCode != 3 {
		t.Errorf("exit code = %d, want 3", out.ExitCode)
	}
	if out.Output != "hello\nworld\n" {
		t.Errorf("output = %q", out.Output)
	}

	tout := ParseExec("RHOST_ERR=timeout\n", token)
	if tout.Err != "timeout" {
		t.Errorf("err = %q, want timeout", tout.Err)
	}

	// The refusal of a pane owned by another program names that program, because
	// the caller's next move depends on what it is.
	busy := ParseExec("RHOST_FG=python3\nRHOST_ERR=busy\n", token)
	if busy.Err != "busy" || busy.Foreground != "python3" {
		t.Errorf("busy refusal = %+v, want err=busy foreground=python3", busy)
	}

	// A command that outlived its deadline is interrupted, and the helper answers
	// with whether the pane actually came back. "timed out" and "timed out but the
	// session is usable" are different instructions to a caller, so the two states
	// must survive parsing.
	if out := ParseExec("RHOST_RECOVERED=1\nRHOST_ERR=timeout\n", token); out.Err != "timeout" || !out.Recovered {
		t.Errorf("recovered timeout parsed as %+v", out)
	}
	if out := ParseExec("RHOST_RECOVERED=0\nRHOST_ERR=timeout\n", token); out.Err != "timeout" || out.Recovered {
		t.Errorf("unrecovered timeout parsed as %+v", out)
	}
	// The marker is only ever printed next to a timeout; a clean command must not
	// pick up a recovered flag out of nowhere.
	if out := ParseExec("RHOST_TOKEN="+token+"\nRHOST_EXIT=0\nRHOST_OUTPUT="+payload+"\n", token); out.Recovered {
		t.Errorf("successful command reports Recovered")
	}
}

func TestParseExecRejectsIncompleteOrForeignResults(t *testing.T) {
	const token = "0123456789abcdef0123456789abcdef"
	validEmpty := "RHOST_TOKEN=" + token + "\nRHOST_EXIT=0\nRHOST_OUTPUT=\n"
	if out := ParseExec(validEmpty, token); out.Err != "" || out.ExitCode != 0 || out.Output != "" {
		t.Fatalf("valid empty stdout parsed as %+v", out)
	}

	cases := map[string]string{
		"empty response":     "",
		"missing token":      "RHOST_EXIT=0\nRHOST_OUTPUT=\n",
		"old marker":         "RHOST_EXIT=0\n\n",
		"wrong token":        "RHOST_TOKEN=ffffffffffffffffffffffffffffffff\nRHOST_EXIT=0\nRHOST_OUTPUT=\n",
		"missing exit":       "RHOST_TOKEN=" + token + "\nRHOST_OUTPUT=\n",
		"negative exit":      "RHOST_TOKEN=" + token + "\nRHOST_EXIT=-1\nRHOST_OUTPUT=\n",
		"large exit":         "RHOST_TOKEN=" + token + "\nRHOST_EXIT=256\nRHOST_OUTPUT=\n",
		"non-numeric exit":   "RHOST_TOKEN=" + token + "\nRHOST_EXIT=nope\nRHOST_OUTPUT=\n",
		"damaged base64":     "RHOST_TOKEN=" + token + "\nRHOST_EXIT=0\nRHOST_OUTPUT=%%%\n",
		"missing output":     "RHOST_TOKEN=" + token + "\nRHOST_EXIT=0\n",
		"duplicate evidence": validEmpty + "RHOST_EXIT=0\n",
	}
	for name, response := range cases {
		t.Run(name, func(t *testing.T) {
			if out := ParseExec(response, token); out.Err != "protocol" || out.ExitCode != -1 {
				t.Errorf("ParseExec(%q) = %+v, want protocol failure with unknown exit", response, out)
			}
		})
	}

	nonzero := "RHOST_TOKEN=" + token + "\nRHOST_EXIT=7\nRHOST_OUTPUT=" +
		base64.StdEncoding.EncodeToString([]byte("failed\n")) + "\n"
	if out := ParseExec(nonzero, token); out.Err != "" || out.ExitCode != 7 || out.Output != "failed\n" {
		t.Errorf("valid non-zero command parsed as %+v", out)
	}
}

func TestParseRead(t *testing.T) {
	payload := base64.StdEncoding.EncodeToString([]byte("abc"))
	out := ParseRead("RHOST_FROM=10\nRHOST_NEXT=13\nRHOST_SIZE=20\n" + payload + "\n")
	if out.Error != "" || out.From != 10 || out.Next != 13 || out.Size != 20 {
		t.Errorf("parsed read = %+v", out)
	}
	if string(out.Data) != "abc" {
		t.Errorf("data = %q", out.Data)
	}
}

func TestParseReadRejectsMalformedHelperResponses(t *testing.T) {
	payload := base64.StdEncoding.EncodeToString([]byte("abc"))
	cases := map[string]string{
		"missing field":       "RHOST_FROM=10\nRHOST_NEXT=13\n" + payload + "\n",
		"duplicate field":     "RHOST_FROM=10\nRHOST_FROM=10\nRHOST_NEXT=13\nRHOST_SIZE=20\n" + payload + "\n",
		"invalid cursor":      "RHOST_FROM=nope\nRHOST_NEXT=13\nRHOST_SIZE=20\n" + payload + "\n",
		"inconsistent cursor": "RHOST_FROM=10\nRHOST_NEXT=21\nRHOST_SIZE=20\n" + payload + "\n",
		"damaged content":     "RHOST_FROM=10\nRHOST_NEXT=13\nRHOST_SIZE=20\n%%%\n",
	}
	for name, response := range cases {
		t.Run(name, func(t *testing.T) {
			if out := ParseRead(response); out.Error != "protocol" {
				t.Errorf("ParseRead(%q) = %+v, want protocol failure", response, out)
			}
		})
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

func TestResolvedSessionHelpersReturnCanonicalID(t *testing.T) {
	for name, script := range map[string]string{
		"exec": ExecScript("dev", "true", time.Second, "token"),
		"read": ReadScript("dev", 0, 0),
		"recover": RecoverScript("dev", time.Second),
	} {
		if !strings.Contains(script, "RHOST_ID=") {
			t.Errorf("%s helper does not return resolved ID", name)
		}
	}
}

func TestCreateScriptValidatesRequestedCwdBeforeCreating(t *testing.T) {
	for _, cwd := range []string{"~", "/home/<user>/project"} {
		s := CreateScript(NewMeta("s_abc", "dev", cwd, "bash"), "bash --noprofile --norc -i")
		if !strings.Contains(s, "RHOST_ERR=invalidcwd") {
			t.Errorf("cwd %q is not validated before create:\n%s", cwd, s)
		}
		if cwd == "~" && !strings.Contains(s, "CWD=\"$HOME\"") {
			t.Errorf("literal ~ is not resolved through remote HOME:\n%s", s)
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
	s := ExecScript("dev", "echo hi", 5*time.Second, "0123456789abcdef0123456789abcdef")
	for _, want := range []string{
		"flock",
		"RHOST_ERR=noflock",
		"RHOST_RESOLVE",
		"133;D;",
		"133;R;0123456789abcdef0123456789abcdef;",
		"sessiondied",
		"RHOST_TOKEN=",
		"RHOST_EXIT=",
		"RHOST_OUTPUT=",
		"stty -echo",
		"paste-buffer -d",
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

// TestExecScriptConfirmsInputSubmission guards the concurrent-session failure
// path: the command and its terminating Enter are one logical write. The Enter
// must be the final newline in the pasted buffer, because a separate send-keys
// may reach the pane before tmux has delivered the paste. Submission must also
// fail explicitly, otherwise a dropped paste is misreported 60 seconds later as
// a user-command timeout.
func TestExecScriptConfirmsInputSubmission(t *testing.T) {
	s := ExecScript("dev", "echo hi", 5*time.Second, "0123456789abcdef0123456789abcdef")
	want := `printf '%s\n' "$cmd" | tmux load-buffer -b "$BUF" - \; paste-buffer -d -b "$BUF" -t "$TMUX:0.0"`
	if !contains(s, want) {
		t.Errorf("ExecScript does not paste command and Enter as one input:\n%s", s)
	}
	start := indexOf(s, `start=$(wc -c`)
	finish := indexOf(s, `rpat=$(printf`)
	if start < 0 || finish < start || contains(s[start:finish], `send-keys`) {
		t.Errorf("ExecScript submits Enter separately from the command paste")
	}
	if !contains(s, `RHOST_ERR=inputfailed`) {
		t.Errorf("ExecScript does not report an input submission failure")
	}
}

// TestExecScriptScansEveryByte is the regression test for a real failure:
// `session exec --command "bash -c 'exit 4'"` timed out on a live host although the
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
	s := ExecScript("dev", "echo hi", 5*time.Second, "0123456789abcdef0123456789abcdef")
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
	for _, want := range []string{"tmp=${seg#\"$rpat\"}", "code=${tmp%%[!0-9]*}"} {
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
	s := ExecScript("dev", "echo hi", 5*time.Second, "0123456789abcdef0123456789abcdef")
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
	probe := indexOf(s, `stty -echo < "$TTY"`)
	paste := indexOf(s, `paste-buffer -d -b "$BUF"`)
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

func TestSendScriptCanPasteThenPressEnter(t *testing.T) {
	s := SendScript("debug", "data-enter", "python3 -i")
	paste := indexOf(s, "tmux paste-buffer")
	enter := indexOf(s, "tmux send-keys -t \"$TMUX:0.0\" Enter")
	if paste < 0 || enter < paste {
		t.Fatalf("data-enter must paste before Enter: paste=%d enter=%d", paste, enter)
	}
}

func TestSessionWritersUseLockAndInvocationLocalBuffers(t *testing.T) {
	for name, s := range map[string]string{
		"exec":    ExecScript("debug", "true", time.Second, "0123456789abcdef0123456789abcdef"),
		"send":    SendScript("debug", "data-enter", "print(1)"),
		"recover": RecoverScript("debug", time.Second),
	} {
		t.Run(name, func(t *testing.T) {
			if !contains(s, `flock -n 9`) {
				t.Errorf("writer does not hold the session lock:\n%s", s)
			}
			if contains(s, "-b rhost_cmd ") || contains(s, "-b rhost_send ") || contains(s, "-b rhost_int ") {
				t.Errorf("writer uses a shared tmux buffer name:\n%s", s)
			}
		})
	}
}

func TestRecoverScriptChecksForegroundAfterInterrupt(t *testing.T) {
	s := RecoverScript("debug", time.Second)
	interrupt := indexOf(s, `send-keys -t "$TMUX:0.0" C-c`)
	foreground := strings.LastIndex(s, `#{pane_current_command}`)
	busy := strings.LastIndex(s, `RHOST_ERR=busy`)
	if interrupt < 0 || foreground < interrupt || busy < foreground {
		t.Fatalf("recover does not interrupt then re-check foreground: interrupt=%d foreground=%d busy=%d\n%s",
			interrupt, foreground, busy, s)
	}
	if !contains(s, `echo "RHOST_FG=$fg"`) {
		t.Errorf("recover does not report the remaining foreground process")
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
