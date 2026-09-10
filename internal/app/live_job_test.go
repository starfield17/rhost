package app

import (
	"encoding/base64"
	"os/exec"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/errs"
)

// TestLiveJob is docs/ARCHITECTURE.md §41 Test C — the job half of "anything
// promised to survive a CLI invocation is owned outside the CLI process"
// (AGENTS.md §4). Every step is its own rhost process, and between them the test
// closes the SSH ControlMaster, so the job cannot be riding on anything the
// starting process left behind: the remote files and the remote process are the
// only state.

func TestLiveJob(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	env := c.mustJSON(t, "--json", "job", "start", host,
		"--name", "livejob", "--cwd", "/var/log", "--env", "RHOST_LIVE_JOB=42",
		"--", "echo starting; pwd; echo $RHOST_LIVE_JOB; sleep 6; echo finished; exit 0")
	id := env.str(t, "id")
	defer cleanupJob(t, c, host, id)
	if id == "" {
		t.Fatal("job start returned no id")
	}
	if got := env.str(t, "state"); got != "running" {
		t.Errorf("job.start state = %q, want running", got)
	}
	if env.num(t, "pid") <= 0 {
		t.Errorf("job.start pid = %d, want a real pid", env.num(t, "pid"))
	}

	// Deliberately tear down the shared transport. The job must not notice: it is
	// a detached remote process, not a child of anything rhost owns.
	c.closeMaster(t, host)

	// A brand-new process rediscovering the job by remote state alone.
	status := c.mustJSON(t, "--json", "job", "status", host, id)
	if got := status.str(t, "state"); got != "running" {
		t.Errorf("after reconnect state = %q, want running", got)
	}
	if got := status.str(t, "cwd"); got != "/var/log" {
		t.Errorf("metadata cwd = %q, want /var/log", got)
	}

	logs := c.mustJSON(t, "--json", "job", "logs", host, id, "--since", "0")
	if got := decodeLog(t, logs); !strings.Contains(got, "starting\n/var/log\n42") {
		t.Errorf("logs after reconnect lost the job's own output: %q", got)
	}

	// Wait for the job to finish on its own; the cursor must then carry the rest.
	deadline := time.Now().Add(2 * time.Minute)
	for {
		s := c.mustJSON(t, "--json", "job", "status", host, id)
		if s.str(t, "state") == "exited" {
			if got := s.num(t, "exit_code"); got != 0 {
				t.Errorf("exit_code = %d, want 0", got)
			}
			if s.str(t, "finished_at") == "" {
				t.Error("finished job recorded no finished_at")
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("job never reached exited: %s", s.str(t, "state"))
		}
		time.Sleep(time.Second)
	}

	full := decodeLog(t, c.mustJSON(t, "--json", "job", "logs", host, id, "--since", "0"))
	if !strings.HasSuffix(full, "finished\n") {
		t.Errorf("job output was truncated at the cursor boundary: %q", full)
	}
	// Everything delivered, nothing pending: `more` must be false once the cursor
	// has caught up with the end of the stream.
	if last := c.mustJSON(t, "--json", "job", "logs", host, id, "--since", "0"); last.bool(t, "more") {
		t.Errorf("more = true for a finished stream read from 0: %s", last.Data)
	}

	// Completed jobs remain inspectable and are listed by a later process
	// (docs/ARCHITECTURE.md §46).
	assertJobListed(t, c, host, id, "exited")

	// A terminal job is not re-stoppable into a different story.
	stop := c.mustJSON(t, "--json", "job", "stop", host, id)
	if got := stop.str(t, "state"); got != "exited" {
		t.Errorf("stop on an exited job changed its state to %q, want exited (a stop request must not rewrite a recorded result)", got)
	}
}

// TestLiveJobLogsCursor pins the incremental read contract of
// docs/ARCHITECTURE.md §24: from/next/more describe byte offsets, an arbitrary
// byte sequence survives the round trip, and a stale offset is clamped instead
// of failing.
func TestLiveJobLogsCursor(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	// A payload with no trailing newline: proof the base64 transport is byte-exact
	// rather than "whatever printf happened to send".
	const payload = "0123456789"
	env := c.mustJSON(t, "--json", "job", "start", host,
		"--", "printf '"+payload+"'; sleep 2; printf 'tail'")
	id := env.str(t, "id")
	defer cleanupJob(t, c, host, id)

	// Read the first half while the job is still running.
	first := waitCursor(t, c, host, id, 10)
	if got := first.num(t, "from"); got != 0 {
		t.Errorf("from = %d, want 0", got)
	}
	if got := string(decodeBytes(t, first)); !strings.HasPrefix(got, payload) {
		t.Errorf("data = %q, want it to start with %q", got, payload)
	}
	if got := first.num(t, "next"); got <= 0 {
		t.Errorf("next = %d, want a positive offset", got)
	}
	next := first.num(t, "next")

	// A read from `next` must not re-deliver those bytes.
	second := c.mustJSON(t, "--json", "job", "logs", host, id, "--since", strconv.Itoa(next))
	if second.num(t, "from") != next {
		t.Errorf("from = %d, want %d", second.num(t, "from"), next)
	}
	if strings.Contains(string(decodeBytes(t, second)), payload) {
		t.Errorf("read from offset %d re-delivered the first chunk", next)
	}

	// An offset past the end is clamped to the end, not an error.
	far := c.mustJSON(t, "--json", "job", "logs", host, id, "--since", "4096")
	if got := far.num(t, "from"); got != far.num(t, "next") {
		t.Errorf("clamped read moved the cursor: from=%d next=%d", far.num(t, "from"), far.num(t, "next"))
	}
	if len(decodeBytes(t, far)) != 0 {
		t.Error("read past the end of the stream delivered data")
	}

	// The whole stream, read in one pass, is byte-exact.
	all := decodeBytes(t, c.mustJSON(t, "--json", "job", "logs", host, id, "--since", "0"))
	if !strings.HasPrefix(string(all), payload) || !strings.Contains(string(all), "tail") {
		t.Errorf("full stream = %q, want %q... plus tail", all, payload)
	}
}

// TestLiveJobStopHarvestsProcessGroup is docs/ARCHITECTURE.md §46's "remote
// process group can be stopped": a job's *children* must go with it, because a
// stop that only killed the wrapper would leave orphans holding the GPU
// (§25's reason for signalling the group).
func TestLiveJobStopHarvestsProcessGroup(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	// A distinctive sleep length, so the survivor check below cannot match some
	// unrelated process on the host.
	marker := 300 + time.Now().Nanosecond()%200

	env := c.mustJSON(t, "--json", "job", "start", host,
		"--name", "groupkill", "--", "echo started; sleep "+strconv.Itoa(marker)+" & sleep "+strconv.Itoa(marker)+"; wait")
	id := env.str(t, "id")
	defer cleanupJob(t, c, host, id)
	pgid := env.num(t, "pid")

	// Both sleeps must be running, in the job's own process group.
	procs := remoteProcs(t, c, host)
	survivors := matchingLines(procs, "sleep "+strconv.Itoa(marker))
	if len(survivors) < 2 {
		t.Fatalf("job children are not running before stop: %v", survivors)
	}
	for _, line := range survivors {
		if !strings.Contains(line, strconv.Itoa(pgid)) {
			t.Errorf("child is not in the job's process group (pgid %d): %s", pgid, line)
		}
	}

	stopped := c.mustJSON(t, "--json", "job", "stop", host, id)
	if got := stopped.str(t, "state"); got != "stopped" {
		t.Errorf("state after stop = %q, want stopped", got)
	}

	// The whole group is gone: no wrapper, no background child, no foreground one.
	if still := matchingLines(remoteProcs(t, c, host), "sleep "+strconv.Itoa(marker)); len(still) != 0 {
		t.Errorf("process group survived `job stop`:\n%s", strings.Join(still, "\n"))
	}

	// A job terminated by signal records that, never a successful 0. This is the
	// reason the wrapper traps TERM: bash would otherwise run its EXIT trap with
	// the status still at its initial 0.
	status := c.mustJSON(t, "--json", "job", "status", host, id)
	if got := status.num(t, "exit_code"); got != 143 {
		t.Errorf("exit_code after stop = %d, want 143 (128+SIGTERM)", got)
	}
	if got := status.str(t, "state"); got != "stopped" {
		t.Errorf("state = %q, want stopped", got)
	}

	// Idempotent: a second stop reports the condition that already holds, and
	// does not rewrite the recorded status.
	again := c.mustJSON(t, "--json", "job", "stop", host, id)
	if got := again.str(t, "state"); got != "stopped" {
		t.Errorf("second stop state = %q, want stopped", got)
	}
	if got := c.mustJSON(t, "--json", "job", "status", host, id).num(t, "exit_code"); got != 143 {
		t.Errorf("second stop rewrote exit_code to %d, want 143", got)
	}
}

// TestLiveJobKillEscalates covers the SIGKILL path: nothing can trap it, so no
// exit code is recorded, and the stopped marker is what makes the state honest.
func TestLiveJobKillEscalates(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	marker := 500 + time.Now().Nanosecond()%200

	env := c.mustJSON(t, "--json", "job", "start", host,
		"--", "trap '' TERM; echo started; sleep "+strconv.Itoa(marker))
	id := env.str(t, "id")
	defer cleanupJob(t, c, host, id)

	// This job ignores SIGTERM — exactly the case `kill` exists for. `job stop`
	// must report that honestly instead of claiming the stop succeeded: the grace
	// period expires with the process still alive, and that is what it says.
	if got := c.mustJSON(t, "--json", "job", "stop", host, id).str(t, "state"); got != "running" {
		t.Errorf("stop on a SIGTERM-ignoring job reported %q, want running", got)
	}

	c.mustJSON(t, "--json", "job", "kill", host, id)
	if still := matchingLines(remoteProcs(t, c, host), "sleep "+strconv.Itoa(marker)); len(still) != 0 {
		t.Errorf("process group survived `job kill`:\n%s", strings.Join(still, "\n"))
	}

	status := c.mustJSON(t, "--json", "job", "status", host, id)
	if got := status.str(t, "state"); got != "stopped" {
		t.Errorf("state after kill = %q, want stopped", got)
	}
	// No exit code was written, and rhost says so rather than inventing one.
	if got := status.num(t, "exit_code"); got != -1 {
		t.Errorf("exit_code after kill = %d, want -1 (nothing recorded)", got)
	}
}

// TestLiveJobFailureIsRecorded checks that a job's own non-zero status survives:
// the stop call afterwards must not relabel a real result as "stopped".
func TestLiveJobFailureIsRecorded(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	env := c.mustJSON(t, "--json", "job", "start", host, "--", "echo about-to-fail; exit 4")
	id := env.str(t, "id")
	defer cleanupJob(t, c, host, id)

	deadline := time.Now().Add(90 * time.Second)
	for {
		s := c.mustJSON(t, "--json", "job", "status", host, id)
		if s.str(t, "state") == "failed" {
			if got := s.num(t, "exit_code"); got != 4 {
				t.Errorf("exit_code = %d, want 4", got)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("job never reached failed: %v", s.Data)
		}
		time.Sleep(time.Second)
	}

	// stderr goes to its own stream, and never into stdout's cursor.
	errEnv := c.mustJSON(t, "--json", "job", "start", host, "--", "echo to-stdout; echo to-stderr >&2; exit 1")
	errID := errEnv.str(t, "id")
	defer cleanupJob(t, c, host, errID)
	waitJobState(t, c, host, errID, "failed")
	out := string(decodeBytes(t, c.mustJSON(t, "--json", "job", "logs", host, errID, "--stream", "stdout", "--since", "0")))
	if !strings.Contains(out, "to-stdout") || strings.Contains(out, "to-stderr") {
		t.Errorf("stdout stream mixed with stderr: %q", out)
	}
	errOut := string(decodeBytes(t, c.mustJSON(t, "--json", "job", "logs", host, errID, "--stream", "stderr", "--since", "0")))
	if !strings.Contains(errOut, "to-stderr") {
		t.Errorf("stderr stream missing its own content: %q", errOut)
	}
}

// TestLiveJobStaleIsNeverSuccess pins the honesty rule of §23: metadata said the
// job was running, the pid is gone, and no exit code was ever written. That is
// `stale`, and it must never be reported as a success.
func TestLiveJobStaleIsNeverSuccess(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	marker := 700 + time.Now().Nanosecond()%200

	env := c.mustJSON(t, "--json", "job", "start", host, "--", "sleep "+strconv.Itoa(marker))
	id := env.str(t, "id")
	defer cleanupJob(t, c, host, id)
	pid := env.num(t, "pid")

	// Kill it behind rhost's back — no stop request, so no stopped marker, and
	// SIGKILL means the wrapper's EXIT trap never ran. This is what a host
	// reboot or a stray OOM kill looks like to a later rhost process.
	c.mustJSON(t, "--json", "exec", host, "--", "kill -9 -"+strconv.Itoa(pid))
	if still := matchingLines(remoteProcs(t, c, host), "sleep "+strconv.Itoa(marker)); len(still) != 0 {
		t.Fatalf("test setup failed, the group survived the direct kill:\n%s", strings.Join(still, "\n"))
	}

	status := c.mustJSON(t, "--json", "job", "status", host, id)
	if got := status.str(t, "state"); got != "stale" {
		t.Errorf("state = %q, want stale", got)
	}
	if got := status.num(t, "exit_code"); got != -1 {
		t.Errorf("exit_code = %d, want -1: nothing was ever recorded", got)
	}
	assertJobListed(t, c, host, id, "stale")
}

// TestLiveJobUnknownIDIsAnError keeps a missing job a stable code rather than an
// empty success: agents branch on error.code (AGENTS.md §6).
func TestLiveJobUnknownIDIsAnError(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	for _, args := range [][]string{
		{"--json", "job", "status", host, "j_nosuchjob"},
		{"--json", "job", "logs", host, "j_nosuchjob", "--since", "0"},
		{"--json", "job", "stop", host, "j_nosuchjob"},
		{"--json", "job", "kill", host, "j_nosuchjob"},
	} {
		c.wantErrorCode(t, errs.JobNotFound, args...)
	}

	// A bad stream name is a usage-time validation failure, before any SSH runs.
	c.wantErrorCode(t, errs.ConfigInvalid, "--json", "job", "logs", host, "j_nosuchjob", "--stream", "syslog")
}

// TestLiveJobNameLookup covers the convenience handle: `--name` is what a human
// types back in, and the id is what an agent keeps. Names may collide, so a
// collision has to be an error rather than a guess.
func TestLiveJobNameLookup(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	// Unique per run: name resolution scans every job on the host, and a leftover
	// from an earlier run would make this ambiguous.
	name := "livename" + strconv.FormatInt(time.Now().UnixNano(), 36)

	first := c.mustJSON(t, "--json", "job", "start", host, "--name", name, "--", "echo by-name; sleep 3")
	id := first.str(t, "id")
	defer cleanupJob(t, c, host, id)

	if got := c.mustJSON(t, "--json", "job", "status", host, name).str(t, "id"); got != id {
		t.Errorf("status by name resolved to %q, want %q", got, id)
	}
	if got := decodeLog(t, c.mustJSON(t, "--json", "job", "logs", host, name, "--since", "0")); !strings.Contains(got, "by-name") {
		t.Errorf("logs by name: %q", got)
	}

	// A second job with the same name makes the handle ambiguous. Both jobs stay
	// addressable by id; the name stops working, loudly.
	second := c.mustJSON(t, "--json", "job", "start", host, "--name", name, "--", "echo collision")
	id2 := second.str(t, "id")
	defer cleanupJob(t, c, host, id2)

	ambiguous := c.wantErrorCode(t, errs.ConfigInvalid, "--json", "job", "status", host, name)
	if ambiguous.Error == nil || !strings.Contains(ambiguous.Error.Message, "id") {
		t.Errorf("ambiguous-name failure must point at the alternative handle: %+v", ambiguous.Error)
	}
	for _, want := range []string{id, id2} {
		if got := c.mustJSON(t, "--json", "job", "status", host, want).str(t, "id"); got != want {
			t.Errorf("status by id %q resolved to %q", want, got)
		}
	}

	// stop by name is refused the same way, so a collision can never signal the
	// wrong job.
	c.wantErrorCode(t, errs.ConfigInvalid, "--json", "job", "stop", host, name)
}

// TestLiveJobHostileHandleNeverExecutes is the regression test for the one place
// jobs interpolated a caller-supplied string into a remote shell script: the job
// handle. A malformed handle must be refused locally, without an SSH round trip,
// and must never leave a trace on the remote host.
func TestLiveJobHostileHandleNeverExecutes(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	marker := "/tmp/rhost-injection-" + strconv.FormatInt(time.Now().UnixNano(), 36)
	hostile := `x"; touch ` + marker + `; #`

	for _, verb := range []string{"status", "logs", "stop", "kill"} {
		c.wantErrorCode(t, errs.ConfigInvalid, "--json", "job", verb, host, hostile)
	}

	check := c.mustJSON(t, "--json", "exec", host, "--", "test -e "+marker+" && echo created || echo absent")
	if got := strings.TrimSpace(check.str(t, "stdout")); got != "absent" {
		t.Errorf("job handle executed shell on the remote host: %s = %q", marker, got)
	}
}

// helpers

// cleanupJob removes the remote state a test created. There is no `job rm` in
// v0.1 (not in §46), so tests use exec on the directory they own.
func cleanupJob(t *testing.T, c liveCLI, host, id string) {
	t.Helper()
	if id == "" {
		return
	}
	if _, stderr, code := c.run(t, "--json", "exec", host, "--",
		`rm -rf "${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost}/jobs/`+id+`"`); code != 0 {
		t.Logf("cleanup job %s: exit=%d %s", id, code, strings.TrimSpace(stderr))
	}
}

// remoteProcs lists pid/ppid/pgid/args on the remote host, for survivor checks.
func remoteProcs(t *testing.T, c liveCLI, host string) string {
	t.Helper()
	return c.mustJSON(t, "--json", "exec", host, "--", "ps -eo pid,ppid,pgid,args").str(t, "stdout")
}

// decodeLog returns the base64 `data` field of a job.logs envelope.
func decodeLog(t *testing.T, env envelope) string {
	t.Helper()
	return string(decodeBytes(t, env))
}

func decodeBytes(t *testing.T, env envelope) []byte {
	t.Helper()
	raw, err := base64.StdEncoding.DecodeString(env.str(t, "data"))
	if err != nil {
		t.Fatalf("data is not valid base64 (%v): %q", err, env.str(t, "data"))
	}
	return raw
}

// waitCursor polls until the log stream has at least n bytes, then returns one
// read from 0. Jobs are asynchronous by contract, so a test that asserts on
// offsets has to wait for content rather than assume it.
func waitCursor(t *testing.T, c liveCLI, host, id string, n int) envelope {
	t.Helper()
	deadline := time.Now().Add(60 * time.Second)
	for {
		env := c.mustJSON(t, "--json", "job", "logs", host, id, "--since", "0")
		if env.num(t, "next") >= n {
			return env
		}
		if time.Now().After(deadline) {
			t.Fatalf("log stream never reached %d bytes (next=%d)", n, env.num(t, "next"))
		}
		time.Sleep(time.Second)
	}
}

func assertJobListed(t *testing.T, c liveCLI, host, id, state string) {
	t.Helper()
	env := c.mustJSON(t, "--json", "job", "list", host)
	var jobs []map[string]interface{}
	env.field(t, "jobs", &jobs)
	for _, j := range jobs {
		if j["id"] == id {
			if j["state"] != state {
				t.Errorf("job %s listed as %v, want %s", id, j["state"], state)
			}
			if pid, _ := j["pid"].(float64); pid <= 0 {
				t.Errorf("job %s listed with pid %v: the list must carry the real pid", id, j["pid"])
			}
			return
		}
	}
	t.Errorf("job %s not rediscovered by job list: %v", id, jobs)
}

func waitJobState(t *testing.T, c liveCLI, host, id string, states ...string) {
	t.Helper()
	deadline := time.Now().Add(90 * time.Second)
	for {
		got := c.mustJSON(t, "--json", "job", "status", host, id).str(t, "state")
		for _, want := range states {
			if got == want {
				return
			}
		}
		if time.Now().After(deadline) {
			t.Fatalf("job %s never reached %v (last seen %q)", id, states, got)
		}
		time.Sleep(time.Second)
	}
}

// closeMaster shuts down the multiplexed connection through OpenSSH itself, so
// the next rhost process must reconnect from scratch. That is the "SSH
// disconnects" half of §20's survival promise.
func (c liveCLI) closeMaster(t *testing.T, host string) {
	t.Helper()
	cmd := exec.Command("ssh", "-o", "ControlPath="+c.controlPath, "-O", "exit", host)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Logf("ssh -O exit %s: %v\n%s", host, err, out) // no master yet is not a failure
	}
	if pid := c.masterPID(t, host); pid > 0 {
		t.Fatalf("ControlMaster still running (pid %d) after close: the test would not prove reconnection", pid)
	}
}
