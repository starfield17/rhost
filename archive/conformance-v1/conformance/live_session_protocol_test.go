package conformance

import (
	"encoding/json"
	"strings"
	"testing"
	"time"
)

// This regression requires Python on the explicitly selected live host.
func TestLiveSessionPythonRecovery(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)
	c.mustJSON(t, "--json", "exec", host, "--command", "command -v python3")
	name := liveName("python")
	marker := "/tmp/" + liveName("notexecuted")
	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	defer c.cleanup(t, "--json", "session", "close", host, name)
	defer c.cleanup(t, "--json", "exec", host, "--command", "rm -f "+marker)
	c.mustJSON(t, "--json", "session", "send", host, name, "--data", "python3 -q", "--enter")
	deadline := time.Now().Add(20 * time.Second)
	for !strings.Contains(c.mustJSON(t, "--json", "session", "read", host, name).str(t, "content"), ">>>") {
		if time.Now().After(deadline) {
			t.Fatal("Python prompt did not appear")
		}
		time.Sleep(livePoll)
	}
	raw, _, _ := c.run(t, "--json", "session", "recover", host, name, "--timeout", "2s")
	var env envelope
	if err := json.Unmarshal([]byte(raw), &env); err != nil {
		t.Fatal(err)
	}
	if env.OK || env.Error == nil || env.Error.Code != "SESSION_BUSY" || env.bool(t, "session_preserved") || !strings.Contains(env.str(t, "foreground"), "python") {
		t.Fatalf("recover must preserve the REPL and report busy: %s", raw)
	}
	for i := 0; i < 3; i++ {
		raw, _, _ = c.run(t, "--json", "session", "exec", host, name, "--command", "touch "+marker)
		if err := json.Unmarshal([]byte(raw), &env); err != nil {
			t.Fatal(err)
		}
		if env.OK || env.Error == nil || env.Error.Code != "SESSION_BUSY" {
			t.Fatalf("exec must refuse Python: %s", raw)
		}
		env.assertUnknownSessionExit(t)
	}
	absent := c.mustJSON(t, "--json", "exec", host, "--command", "test ! -e "+marker)
	if absent.num(t, "exit_code") != 0 {
		t.Fatal("refused command created its marker")
	}
	c.mustJSON(t, "--json", "session", "send", host, name, "--data", "exit()", "--enter")
	deadline = time.Now().Add(20 * time.Second)
	for {
		raw, _, _ = c.run(t, "--json", "session", "exec", host, name, "--command", "touch "+marker)
		if err := json.Unmarshal([]byte(raw), &env); err != nil {
			t.Fatal(err)
		}
		if env.OK {
			break
		}
		if env.Error == nil || env.Error.Code != "SESSION_BUSY" || time.Now().After(deadline) {
			t.Fatalf("exec after Python exit failed: %s", raw)
		}
		time.Sleep(livePoll)
	}
	exists := c.mustJSON(t, "--json", "exec", host, "--command", "test -e "+marker)
	if exists.num(t, "exit_code") != 0 {
		t.Fatal("successful command did not create its marker")
	}
}

func TestLiveSessionConcurrentInputIsolation(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	names := []string{liveName("panea"), liveName("paneb")}
	for _, name := range names {
		c.mustJSON(t, "--json", "session", "create", host, "--name", name)
		t.Cleanup(func() { c.cleanup(t, "--json", "session", "close", host, name) })
	}
	for i, name := range names {
		t.Run(name, func(t *testing.T) {
			t.Parallel()
			for n := 0; n < 5; n++ {
				result := c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "printf '%s\\n' "+name)
				output := result.str(t, "stdout")
				if !strings.Contains(output, name) || strings.Contains(output, names[1-i]) {
					t.Fatalf("cross-pane output: %q", output)
				}
			}
		})
	}
}
