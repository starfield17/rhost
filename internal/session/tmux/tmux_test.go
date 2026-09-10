package tmux

import (
	"encoding/base64"
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
