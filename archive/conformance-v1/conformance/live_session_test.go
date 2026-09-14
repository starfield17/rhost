package conformance

import (
	"bytes"
	"encoding/json"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"testing"
	"time"
)

// TestLiveSession is docs/architecture/engineering.md: shell state must survive
// separate rhost *processes*, because the session is owned by remote tmux and
// the CLI owns nothing durable (AGENTS.md §4). Every step below is its own
// child process; nothing shares memory with the previous one.

func TestLiveSession(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)
	name := liveName("sess")

	env := c.mustJSON(t, "--json", "session", "create", host, "--name", name, "--cwd", "/tmp")
	canonicalID := env.str(t, "session_id")
	if canonicalID == "" {
		t.Fatal("session.create returned no id")
	}
	defer func() {
		_, stderr, code := c.run(t, "--json", "session", "close", host, name)
		if code != 0 {
			t.Errorf("cleanup session close: exit=%d %s", code, strings.TrimSpace(stderr))
		}
	}()

	// A caller name must not replace the canonical identity in subsequent results.
	identified := c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "true")
	if identified.str(t, "session_id") != canonicalID {
		t.Fatal("session exec replaced canonical identity with caller reference")
	}
	if identified.str(t, "session_ref") != name {
		t.Fatal("session exec lost caller reference")
	}
	// cwd persists across process boundaries.

	c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "cd /var/log && echo moved")
	got := c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "pwd").str(t, "stdout")
	if strings.TrimSpace(got) != "/var/log" {
		t.Errorf("cwd did not survive a process exit: got %q, want /var/log", strings.TrimSpace(got))
	}

	// Environment persists too, and does so under the *same* shell process.
	c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "export RHOST_LIVE_ENV=42")
	got = c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "echo $RHOST_LIVE_ENV").str(t, "stdout")
	if strings.TrimSpace(got) != "42" {
		t.Errorf("env did not survive a process exit: got %q, want 42", strings.TrimSpace(got))
	}

	// The remote shell pid is stable across invocations: it is the same tmux pane.
	shellPID := func() string {
		return strings.TrimSpace(c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "echo $$").str(t, "stdout"))
	}
	if first, second := shellPID(), shellPID(); first == "" || first != second {
		t.Errorf("session shell changed between calls: %q -> %q", first, second)
	}

	// session exec reports the command's real exit status, in the envelope and in
	// the process status. It must be a subshell: a bare `exit N` closes the
	// session's own shell, which is the documented SESSION_NOT_FOUND path and is
	// covered by TestLiveSessionExitIsReported.
	stdout, _, exit := c.run(t, "--json", "session", "exec", host, name, "--command", "bash -c 'exit 4'")
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
	c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "echo "+token)

	first := c.mustJSON(t, "--json", "session", "read", host, name, "--since", "0")
	if !strings.Contains(first.str(t, "content"), token) {
		t.Fatalf("read --since 0 did not contain %q: %q", token, first.str(t, "content"))
	}
	next := first.num(t, "next")
	if next <= 0 {
		t.Fatalf("cursor did not advance: next=%d", next)
	}

	second := c.mustJSON(t, "--json", "session", "read", host, name, "--since", strconv.Itoa(next))
	if strings.Contains(second.str(t, "content"), token) {
		t.Errorf("read --since %d re-delivered the token", next)
	}
}

// testLocalDeathDoesNotKillTheSession SIGKILLs the local CLI mid-command and
// then proves (a) the session is still discoverable, (b) the *remote* command
// kept running — it still holds the writer lock, so a second writer is refused
// with SESSION_UNHEALTHY rather than interleaving into the pane.
func testLocalDeathDoesNotKillTheSession(t *testing.T, c liveCLI, host, name string) {
	t.Helper()
	// SIGKILL of the local CLI must not take the remote helper with it
	// (AGENTS.md §4): the helper is an SSH child, not a child of rhost, and it
	// keeps the writer lock until the command ends. The next session exec then
	// hits a non-blocking flock and is refused with SESSION_UNHEALTHY rather
	// than interleaving into the pane.
	cmd, out, started := c.start(t, "--json", "session", "exec", host, name, "--command", "sleep 8")
	time.Sleep(4 * time.Second)
	if err := cmd.Process.Kill(); err != nil {
		t.Fatalf("kill local rhost process: %v", err)
	}
	_ = cmd.Wait() // reaping only: a killed process has no meaningful status
	recordLiveCall(time.Since(started))
	t.Logf("killed local CLI mid-command; captured stdout=%q", out.String())

	busy := c.wantErrorCode(t, "SESSION_UNHEALTHY",
		"--json", "session", "exec", host, name, "--command", "echo should-be-refused")
	if !busy.Error.Retryable {
		t.Errorf("SESSION_UNHEALTHY from a busy writer must be retryable: %+v", busy.Error)
	}
	assertRediscoverable(t, c, host, name)

	// Once the remote command finishes on its own, the same session works again.
	deadline := time.Now().Add(2 * time.Minute)
	for {
		stdout, _, exit := c.run(t, "--json", "session", "exec", host, name, "--command", "echo alive-again")
		if exit == 0 && strings.Contains(stdout, "alive-again") {
			return
		}
		if time.Now().After(deadline) {
			t.Fatal("session never became usable again after the local process died")
		}
		time.Sleep(livePoll)
	}
}

func TestLiveSessionBoundaryIsReliable(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)
	name := liveName("bound")

	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	defer c.cleanup(t, "--json", "session", "close", host, name)

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
			env := c.mustJSON(t, "--json", "session", "exec", host, name, "--command", tc.command)
			if got := env.num(t, "exit_code"); got != tc.want {
				t.Fatalf("round %d: %q exit_code = %d, want %d (%s)", round, tc.command, got, tc.want, env.Data)
			}
		}
	}

	// A boundary miss surfaces as a timeout, and a timeout must still leave the
	// session usable: check the shell is alive after the whole sequence.
	if got := strings.TrimSpace(c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "echo still-here").str(t, "stdout")); !strings.Contains(got, "still-here") {
		t.Errorf("session unusable after the boundary sequence: %q", got)
	}
}

// TestLiveSessionSendRawInput covers the raw-injection path agents use for REPLs.
func TestLiveSessionSendRawInput(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)
	name := liveName("send")

	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	defer c.cleanup(t, "--json", "session", "close", host, name)

	c.mustJSON(t, "--json", "session", "send", host, name, "--data", "echo sent-ok", "--enter")
	deadline := time.Now().Add(20 * time.Second)
	for {
		env := c.mustJSON(t, "--json", "session", "read", host, name, "--since", "0")
		if strings.Contains(env.str(t, "content"), "sent-ok") {
			return
		}
		if time.Now().After(deadline) {
			t.Fatal("injected input never produced output in the session log")
		}
		time.Sleep(livePoll)
	}
}

// TestLiveSessionBusyRefusesExec is the §16 contract on a real pane: once a
// program owns the terminal, `session exec` refuses with SESSION_BUSY instead of
// pasting a managed command into that program, `session send` stays the raw path,
// and `session recover` is what brings the shell back.
func TestLiveSessionBusyRefusesExec(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)
	name := liveName("busy")

	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	defer c.cleanup(t, "--json", "session", "close", host, name)

	// Occupy the pane with a program that reads the terminal: the exact shape that
	// used to swallow the next exec's probe and payload.
	c.mustJSON(t, "--json", "session", "send", host, name, "--data", "cat\n")

	// The program starts asynchronously, so the refusal is what we wait for: a
	// successful exec before `cat` takes over is not a failure of the check.
	deadline := time.Now().Add(20 * time.Second)
	var busy envelope
	for {
		stdout, stderr, exit := c.run(t, "--json", "session", "exec", host, name, "--command", "echo must-not-run")
		var env envelope
		if err := json.Unmarshal([]byte(strings.TrimSpace(stdout)), &env); err == nil && !env.OK {
			if env.Error == nil || env.Error.Code != "SESSION_BUSY" {
				t.Fatalf("exec on a busy pane: error = %+v, want SESSION_BUSY\nstdout=%q stderr=%q exit=%d",
					env.Error, stdout, stderr, exit)
			}
			if !env.Error.Retryable {
				t.Errorf("SESSION_BUSY must be retryable (the caller can interrupt the program)")
			}
			busy = env
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("the pane never became busy with `cat`; last exec said %q", stdout)
		}
		time.Sleep(livePoll)
	}
	if msg := busy.Error.Message; !strings.Contains(msg, "cat") {
		t.Errorf("SESSION_BUSY must name what owns the pane, got %q", msg)
	}
	// The refused command must not have run: its output cannot be in the log.
	if got := c.mustJSON(t, "--json", "session", "read", host, name, "--since", "0").str(t, "content"); strings.Contains(got, "must-not-run") {
		t.Errorf("a refused exec ran anyway: %q", got)
	}

	// recover interrupts the program and proves the shell answers again.
	recovered := c.mustJSON(t, "--json", "session", "recover", host, name)
	if !recovered.bool(t, "session_preserved") {
		t.Fatal("session recover did not bring the shell back")
	}
	after := c.mustJSON(t, "--json", "session", "exec", host, name, "--command", "echo back-ok")
	if got := strings.TrimSpace(after.str(t, "stdout")); got != "back-ok" {
		t.Errorf("exec after recover = %q, want back-ok", got)
	}
}

// TestLiveSessionExitIsReported pins the documented behaviour: running `exit`
// inside a session ends it, and the adapter says so with a stable code.
func TestLiveSessionExitIsReported(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)
	name := liveName("exit")

	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	defer c.cleanup(t, "--json", "session", "close", host, name)

	c.wantErrorCode(t, "SESSION_NOT_FOUND", "--json", "session", "exec", host, name, "--command", "exit 7")
}

// TestLiveSessionRejectsUnknownTarget checks a missing session is a stable code
// and never a silent success.
func TestLiveSessionRejectsUnknownTarget(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)
	c.wantErrorCode(t, "SESSION_NOT_FOUND", "--json", "session", "exec", host, "nosuchsession", "--command", "true")
}

// start launches a CLI process without waiting on it, for kill-the-client tests.
func (c liveCLI) start(t *testing.T, args ...string) (*exec.Cmd, *bytes.Buffer, time.Time) {
	t.Helper()
	cmd := exec.Command(c.bin, args...)
	cmd.Env = append(os.Environ(), c.env...)
	cmd.Dir = c.dir
	var out bytes.Buffer
	cmd.Stdout, cmd.Stderr = &out, &out
	started := time.Now()
	if err := cmd.Start(); err != nil {
		t.Fatalf("start %v: %v", args, err)
	}
	return cmd, &out, started
}
