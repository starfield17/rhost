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
	} {
		if !contains(s, want) {
			t.Errorf("CreateScript missing %q", want)
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
		"RHOST_RESOLVE",
		"133;D;",
		"sessiondied",
		"RHOST_EXIT=",
		"stty -echo",
		"tmux paste-buffer",
	} {
		if !contains(s, want) {
			t.Errorf("ExecScript missing %q", want)
		}
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
