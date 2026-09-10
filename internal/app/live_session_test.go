package app

import (
	"bytes"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/errs"
)

// TestLiveSession is docs/ARCHITECTURE.md §41 Test B: shell state must survive
// separate rhost *processes*, because the session is owned by remote tmux and
// the CLI owns nothing durable (AGENTS.md §4). Every step below is its own
// child process; nothing shares memory with the previous one.

func TestLiveSession(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	const name = "livestate"

	env := c.mustJSON(t, "--json", "session", "create", host, "--name", name, "--cwd", "/tmp")
	if id := env.str(t, "id"); id == "" {
		t.Fatal("session.create returned no id")
	}
	defer func() {
		_, stderr, code := c.run(t, "--json", "session", "close", host, name)
		if code != 0 {
			t.Logf("cleanup session close: exit=%d %s", code, strings.TrimSpace(stderr))
		}
	}()

	// cwd persists across process boundaries.
	c.mustJSON(t, "--json", "session", "exec", host, name, "--", "cd /var/log && echo moved")
	got := c.mustJSON(t, "--json", "session", "exec", host, name, "--", "pwd").str(t, "output")
	if strings.TrimSpace(got) != "/var/log" {
		t.Errorf("cwd did not survive a process exit: got %q, want /var/log", strings.TrimSpace(got))
	}

	// Environment persists too, and does so under the *same* shell process.
	c.mustJSON(t, "--json", "session", "exec", host, name, "--", "export RHOST_LIVE_ENV=42")
	got = c.mustJSON(t, "--json", "session", "exec", host, name, "--", "echo $RHOST_LIVE_ENV").str(t, "output")
	if strings.TrimSpace(got) != "42" {
		t.Errorf("env did not survive a process exit: got %q, want 42", strings.TrimSpace(got))
	}

	// The remote shell pid is stable across invocations: it is the same tmux pane.
	shellPID := func() string {
		return strings.TrimSpace(c.mustJSON(t, "--json", "session", "exec", host, name, "--", "echo $$").str(t, "output"))
	}
	if first, second := shellPID(), shellPID(); first == "" || first != second {
		t.Errorf("session shell changed between calls: %q -> %q", first, second)
	}

	// session exec reports the command's real exit status, in the envelope and in
	// the process status. It must be a subshell: a bare `exit N` closes the
	// session's own shell, which is the documented SESSION_NOT_FOUND path and is
	// covered by TestLiveSessionExitIsReported.
	stdout, _, exit := c.run(t, "--json", "session", "exec", host, name, "--", "bash -c 'exit 4'")
	if exit != 4 {
		t.Errorf("session exec exit = %d, want 4\n%s", exit, stdout)
	}
	if got := strings.TrimSpace(stdout); !strings.Contains(got, `"exit_code":4`) {
		t.Errorf("data.exit_code should carry 4: %s", got)
	}

	assertRediscoverable(t, c, host, name)
	testReadCursor(t, c, host, name)
	testLocalDeathDoesNotKillTheSession(t, c, host, name)
}

// assertRediscoverable proves a session is found by state on the remote host,
// not by anything the creating process remembered.
func assertRediscoverable(t *testing.T, c liveCLI, host, name string) {
	t.Helper()
	env := c.mustJSON(t, "--json", "session", "list", host)
	var sessions []map[string]interface{}
	env.field(t, "sessions", &sessions)
	for _, s := range sessions {
		if s["name"] == name && s["status"] == "alive" {
			return
		}
	}
	t.Errorf("session %q not rediscovered via session list: %v", name, sessions)
}

// testReadCursor proves the incremental log cursor: a read from 0 advances, and
// a read from the returned offset does not re-deliver earlier bytes.
func testReadCursor(t *testing.T, c liveCLI, host, name string) {
	t.Helper()
	const token = "cursor-probe"
	c.mustJSON(t, "--json", "session", "exec", host, name, "--", "echo "+token)

	first := c.mustJSON(t, "--json", "session", "read", host, name, "--since", "0")
	if !strings.Contains(first.str(t, "data"), token) {
		t.Fatalf("read --since 0 did not contain %q: %q", token, first.str(t, "data"))
	}
	next := first.num(t, "next")
	if next <= 0 {
		t.Fatalf("cursor did not advance: next=%d", next)
	}

	second := c.mustJSON(t, "--json", "session", "read", host, name, "--since", strconv.Itoa(next))
	if strings.Contains(second.str(t, "data"), token) {
		t.Errorf("read --since %d re-delivered the token", next)
	}
}

// testLocalDeathDoesNotKillTheSession SIGKILLs the local CLI mid-command and
// then proves (a) the session is still discoverable, (b) the *remote* command
// kept running — it still holds the writer lock, so a second writer is refused
// with SESSION_UNHEALTHY rather than interleaving into the pane.
func testLocalDeathDoesNotKillTheSession(t *testing.T, c liveCLI, host, name string) {
	t.Helper()
	cmd, out := c.start(t, "--json", "session", "exec", host, name, "--", "sleep 60")
	time.Sleep(4 * time.Second)
	if err := cmd.Process.Kill(); err != nil {
		t.Fatalf("kill local rhost process: %v", err)
	}
	_ = cmd.Wait() // reaping only: a killed process has no meaningful status
	t.Logf("killed local CLI mid-command; captured stdout=%q", out.String())

	assertRediscoverable(t, c, host, name)

	busy := c.wantErrorCode(t, errs.SessionUnhealthy,
		"--json", "session", "exec", host, name, "--", "echo should-be-refused")
	if !busy.Error.Retryable {
		t.Errorf("SESSION_UNHEALTHY from a busy writer must be retryable: %+v", busy.Error)
	}

	// Once the remote command finishes on its own, the same session works again.
	deadline := time.Now().Add(2 * time.Minute)
	for {
		stdout, _, exit := c.run(t, "--json", "session", "exec", host, name, "--", "echo alive-again")
		if exit == 0 && strings.Contains(stdout, "alive-again") {
			return
		}
		if time.Now().After(deadline) {
			t.Fatal("session never became usable again after the local process died")
		}
		time.Sleep(5 * time.Second)
	}
}

// TestLiveSessionBoundaryIsReliable is the regression test for a marker race that
// made `session exec -- "bash -c 'exit 4'"` time out on a real host even though
// the shell had already reported 4. Commands that finish quietly are the risky
// shape — there is no later output to drag the scan forward — so this repeats the
// exact form many times, across the codes, and fails on any missed boundary.
func TestLiveSessionBoundaryIsReliable(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	const name = "livebound"

	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	defer c.run(t, "--json", "session", "close", host, name)

	cases := []struct {
		command string
		want    int
	}{
		{"bash -c 'exit 4'", 4},
		{"bash -c 'exit 0'", 0},
		{"bash -c 'exit 7'", 7},
		{"sh -c 'exit 9'", 9},
		{"( exit 11 )", 11},
		{"true", 0},
		{"false", 1},
		{"bash -c 'echo out; exit 3'", 3},
	}

	// Two passes: the race is a timing accident, so one pass proves little.
	for round := 0; round < 2; round++ {
		for _, tc := range cases {
			env := c.mustJSON(t, "--json", "session", "exec", host, name, "--", tc.command)
			if got := env.num(t, "exit_code"); got != tc.want {
				t.Fatalf("round %d: %q exit_code = %d, want %d (%s)", round, tc.command, got, tc.want, env.Data)
			}
		}
	}

	// A boundary miss surfaces as a timeout, and a timeout must still leave the
	// session usable: check the shell is alive after the whole sequence.
	if got := strings.TrimSpace(c.mustJSON(t, "--json", "session", "exec", host, name, "--", "echo still-here").str(t, "output")); !strings.Contains(got, "still-here") {
		t.Errorf("session unusable after the boundary sequence: %q", got)
	}
}

// TestLiveSessionSendRawInput covers the raw-injection path agents use for REPLs.
func TestLiveSessionSendRawInput(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	const name = "livesend"

	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	defer c.run(t, "--json", "session", "close", host, name)

	c.mustJSON(t, "--json", "session", "send", host, name, "--data", "echo sent-ok\n")
	deadline := time.Now().Add(20 * time.Second)
	for {
		env := c.mustJSON(t, "--json", "session", "read", host, name, "--since", "0")
		if strings.Contains(env.str(t, "data"), "sent-ok") {
			return
		}
		if time.Now().After(deadline) {
			t.Fatal("injected input never produced output in the session log")
		}
		time.Sleep(2 * time.Second)
	}
}

// TestLiveSessionExitIsReported pins the documented behaviour: running `exit`
// inside a session ends it, and the adapter says so with a stable code.
func TestLiveSessionExitIsReported(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	const name = "liveexit"

	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	defer c.run(t, "--json", "session", "close", host, name)

	c.wantErrorCode(t, errs.SessionNotFound, "--json", "session", "exec", host, name, "--", "exit 7")
}

// TestLiveSessionRejectsUnknownTarget checks a missing session is a stable code
// and never a silent success.
func TestLiveSessionRejectsUnknownTarget(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	c.wantErrorCode(t, errs.SessionNotFound, "--json", "session", "exec", host, "nosuchsession", "--", "true")
}

// start launches a CLI process without waiting on it, for kill-the-client tests.
func (c liveCLI) start(t *testing.T, args ...string) (*exec.Cmd, *bytes.Buffer) {
	t.Helper()
	cmd := exec.Command(c.bin, args...)
	cmd.Env = append(os.Environ(), "RHOST_CACHE_DIR="+c.cache)
	var out bytes.Buffer
	cmd.Stdout, cmd.Stderr = &out, &out
	if err := cmd.Start(); err != nil {
		t.Fatalf("start %v: %v", args, err)
	}
	return cmd, &out
}
