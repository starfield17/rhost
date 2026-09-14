package app

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/config"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/shell"
)

// Live tests drive the *built binary* as a child process, never the Go API
// directly. That is the whole point: rhost owns nothing durable, so anything
// promised to survive a CLI invocation can only be proven by exiting the
// process and starting a new one (AGENTS.md §4, docs/architecture/engineering.md).
//
// They are opt-in and take their target from the environment, so this file runs
// unchanged on any machine:
//
//	RHOST_TEST_LIVE=1 RHOST_TEST_HOST=<user>@<host> go test ./internal/app/ -run Live -v
//
// docs/architecture/engineering.md (network interruption) is deliberately not
// automated here: it needs a human to cut the link, so it stays a manual step.

// liveBinDir holds the throwaway binary built once per live run.
var liveBinDir string

// liveCacheDirs are a small pool of ControlMaster namespaces. A parallel test
// leases one namespace for its lifetime, so its own CLI calls reuse a connection
// without opening concurrent channels on the same master as another test.
var liveCacheDirs []string
var liveCachePool chan string

var liveHarness struct {
	sync.Mutex
	remoteDirs []string
	calls      int
	total      time.Duration
	max        time.Duration
	started    time.Time
}

var liveResourceMu sync.Mutex
var liveNameSeq atomic.Uint64

// livePoll is the wait between live-test observations. Persistence is proven
// by a later CLI process, not by sleeping a full second between polls.
const livePoll = 200 * time.Millisecond
const liveParallelism = 3

func TestMain(m *testing.M) {
	code := m.Run()
	if err := cleanupLiveResources(); err != nil {
		fmt.Fprintf(os.Stderr, "live cleanup: %v\n", err)
		code = 1
	}
	liveHarness.Lock()
	if liveHarness.calls > 0 {
		fmt.Fprintf(os.Stderr, "live stats: calls=%d cli_time=%s max_call=%s wall=%s\n",
			liveHarness.calls, liveHarness.total.Round(time.Millisecond),
			liveHarness.max.Round(time.Millisecond), time.Since(liveHarness.started).Round(time.Millisecond))
	}
	liveHarness.Unlock()
	if liveBinDir != "" {
		_ = os.RemoveAll(liveBinDir)
	}
	for _, dir := range liveCacheDirs {
		_ = os.RemoveAll(dir)
	}
	os.Exit(code)
}

// liveName returns a unique session name for this test so leftovers from an
// earlier run cannot collide. It stays inside the session name character class.
func liveName(prefix string) string {
	return fmt.Sprintf("rlive-%s-%d-%d", prefix, time.Now().UnixNano(), liveNameSeq.Add(1))
}

// liveCLI is one rhost invocation environment: a binary plus an RHOST_CACHE_DIR
// that selects the ControlMaster namespace.
type liveCLI struct {
	bin   string
	cache string
	env   []string
	dir   string
	// controlPath is resolved through internal/config, the same code the child
	// process runs, so the parent never duplicates rhost's path arithmetic.
	controlPath string
}

// deepCacheRoot builds a deliberately long cache root: the shape that made
// OpenSSH refuse every command with "ControlPath too long".
func deepCacheRoot(t *testing.T) string {
	t.Helper()
	root := filepath.Join(t.TempDir(), strings.Repeat("deep-dir-", 8), "Library", "Caches", "rhost")
	if err := os.MkdirAll(root, 0o700); err != nil {
		t.Fatalf("deep cache root: %v", err)
	}
	return root
}

// envelope mirrors schemas/result-v1.schema.json. Tests assert on codes and
// fields, never on English message text, matching what agents are told to do.
type envelope struct {
	SchemaVersion int             `json:"schema_version"`
	Operation     string          `json:"operation"`
	OK            bool            `json:"ok"`
	Host          string          `json:"host"`
	Data          json.RawMessage `json:"data"`
	Error         *struct {
		Code      string `json:"code"`
		Message   string `json:"message"`
		Retryable bool   `json:"retryable"`
	} `json:"error"`
}

// liveHost returns the caller-supplied target, skipping unless the suite was
// explicitly enabled. Nothing about any specific machine is hardcoded here.
func liveHost(t *testing.T) string {
	t.Helper()
	if os.Getenv("RHOST_TEST_LIVE") != "1" {
		t.Skip("set RHOST_TEST_LIVE=1 to run live tests")
	}
	host := os.Getenv("RHOST_TEST_HOST")
	if host == "" {
		t.Fatal("RHOST_TEST_LIVE=1 but RHOST_TEST_HOST is unset: name your own target")
	}
	return host
}

// cli builds rhost once for the run and leases one ControlMaster namespace from
// the suite pool. Each test reuses its connection across child processes, while
// parallel tests never contend for channels on the same master.
func cli(t *testing.T) liveCLI {
	t.Helper()
	return withCacheDir(t, liveCLI{bin: buildBinary(t)}, pooledCacheDir(t))
}

// cliIsolated is cli with a private ControlMaster namespace. Use it when the
// test closes the master or counts sockets in the cache dir.
func cliIsolated(t *testing.T) liveCLI {
	t.Helper()
	dir, err := os.MkdirTemp("", "rhost-live-*")
	if err != nil {
		t.Fatalf("cache dir: %v", err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	return withCacheDir(t, liveCLI{bin: buildBinary(t)}, dir)
}

func pooledCacheDir(t *testing.T) string {
	t.Helper()
	liveResourceMu.Lock()
	if liveCachePool == nil {
		liveCachePool = make(chan string, liveParallelism)
		for range liveParallelism {
			dir, err := os.MkdirTemp("", "rhost-live-cache-*")
			if err != nil {
				liveResourceMu.Unlock()
				t.Fatalf("cache pool: %v", err)
			}
			liveCacheDirs = append(liveCacheDirs, dir)
			liveCachePool <- dir
		}
	}
	pool := liveCachePool
	liveResourceMu.Unlock()
	dir := <-pool
	t.Cleanup(func() { pool <- dir })
	return dir
}

// withCacheDir re-points a CLI at another cache root and resolves the socket
// template exactly as the child process will.
func withCacheDir(t *testing.T, c liveCLI, dir string) liveCLI {
	t.Helper()
	c.cache = dir
	c.env = append(c.env, "RHOST_CACHE_DIR="+dir)
	c.controlPath = config.ControlPathForCache(dir)
	return c
}

func (c liveCLI) withEnv(values ...string) liveCLI {
	c.env = append(append([]string{}, c.env...), values...)
	return c
}

func (c liveCLI) withDir(dir string) liveCLI {
	c.dir = dir
	return c
}

// buildBinary compiles the CLI under test once per live run.
func buildBinary(t *testing.T) string {
	t.Helper()
	liveResourceMu.Lock()
	defer liveResourceMu.Unlock()
	if liveBinDir == "" {
		dir, err := os.MkdirTemp("", "rhost-bin")
		if err != nil {
			t.Fatalf("temp dir: %v", err)
		}
		liveBinDir = dir
	}
	bin := filepath.Join(liveBinDir, "rhost")
	if _, err := os.Stat(bin); err == nil {
		return bin
	}
	build := exec.Command("go", "build", "-o", bin, "./cmd/rhost")
	build.Dir = filepath.Join("..", "..")
	if out, err := build.CombinedOutput(); err != nil {
		t.Fatalf("go build ./cmd/rhost: %v\n%s", err, out)
	}
	return bin
}

// run executes one rhost process and returns its exit status and streams.
func (c liveCLI) run(t *testing.T, args ...string) (stdout, stderr string, code int) {
	return c.runInput(t, "", args...)
}

func (c liveCLI) runInput(t *testing.T, input string, args ...string) (stdout, stderr string, code int) {
	t.Helper()
	cmd := exec.Command(c.bin, args...)
	cmd.Env = append(os.Environ(), c.env...)
	cmd.Dir = c.dir
	cmd.Stdin = strings.NewReader(input)
	var so, se bytes.Buffer
	cmd.Stdout, cmd.Stderr = &so, &se
	started := time.Now()
	err := cmd.Run()
	recordLiveCall(time.Since(started))
	code = 0
	var ee *exec.ExitError
	switch {
	case err == nil:
	case errors.As(err, &ee):
		code = ee.ExitCode()
	default:
		t.Fatalf("run %v: %v", args, err)
	}
	return so.String(), se.String(), code
}

func recordLiveCall(elapsed time.Duration) {
	liveHarness.Lock()
	defer liveHarness.Unlock()
	if liveHarness.calls == 0 {
		liveHarness.started = time.Now().Add(-elapsed)
	}
	liveHarness.calls++
	liveHarness.total += elapsed
	if elapsed > liveHarness.max {
		liveHarness.max = elapsed
	}
}

func registerLiveRemoteDir(dir string) {
	liveHarness.Lock()
	defer liveHarness.Unlock()
	liveHarness.remoteDirs = append(liveHarness.remoteDirs, dir)
}

func cleanupLiveResources() error {
	liveHarness.Lock()
	dirs := append([]string{}, liveHarness.remoteDirs...)
	liveHarness.Unlock()
	if len(dirs) == 0 || liveBinDir == "" {
		return nil
	}
	cacheDir := ""
	if len(liveCacheDirs) > 0 {
		cacheDir = liveCacheDirs[0]
	}
	if cacheDir == "" {
		var err error
		cacheDir, err = os.MkdirTemp("", "rhost-live-cleanup-*")
		if err != nil {
			return fmt.Errorf("create cache directory: %w", err)
		}
		defer os.RemoveAll(cacheDir)
	}
	parts := make([]string, 0, 1)
	if len(dirs) > 0 {
		var removeDirs strings.Builder
		removeDirs.WriteString("rm -rf --")
		for _, dir := range dirs {
			removeDirs.WriteByte(' ')
			removeDirs.WriteString(shell.Quote(dir))
		}
		parts = append(parts, removeDirs.String())
	}
	cmd := exec.Command(filepath.Join(liveBinDir, "rhost"), "exec", os.Getenv("RHOST_TEST_HOST"), "--command", strings.Join(parts, "; "))
	cmd.Env = append(os.Environ(), "RHOST_CACHE_DIR="+cacheDir)
	started := time.Now()
	out, err := cmd.CombinedOutput()
	recordLiveCall(time.Since(started))
	if err != nil {
		return fmt.Errorf("remove remote test directories: %w: %s", err, strings.TrimSpace(string(out)))
	}
	return nil
}

// mustJSON runs a command that must succeed and decodes its envelope.
func (c liveCLI) mustJSON(t *testing.T, args ...string) envelope {
	t.Helper()
	stdout, stderr, code := c.run(t, args...)
	var env envelope
	if err := json.Unmarshal([]byte(strings.TrimSpace(stdout)), &env); err != nil {
		t.Fatalf("%v: stdout is not one JSON document (%v)\nstdout=%q\nstderr=%q\nexit=%d",
			args, err, stdout, stderr, code)
	}
	if env.Error != nil {
		t.Fatalf("%v failed: %s: %s", args, env.Error.Code, env.Error.Message)
	}
	if !env.OK {
		t.Fatalf("%v returned ok=false with no error payload: %s", args, stdout)
	}
	return env
}

func (e envelope) field(t *testing.T, key string, dst interface{}) {
	t.Helper()
	if len(e.Data) == 0 {
		t.Fatalf("envelope has no data for key %q", key)
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal(e.Data, &m); err != nil {
		t.Fatalf("data is not an object: %v", err)
	}
	raw, ok := m[key]
	if !ok {
		t.Fatalf("data has no %q key: %s", key, e.Data)
	}
	if err := json.Unmarshal(raw, dst); err != nil {
		t.Fatalf("decode data.%s: %v", key, err)
	}
}

func (e envelope) str(t *testing.T, key string) string {
	t.Helper()
	var s string
	e.field(t, key, &s)
	return s
}

func (e envelope) num(t *testing.T, key string) int {
	t.Helper()
	var n int
	e.field(t, key, &n)
	return n
}

func (e envelope) bool(t *testing.T, key string) bool {
	t.Helper()
	var b bool
	e.field(t, key, &b)
	return b
}

// wantErrorCode asserts the machine-readable failure code, the only thing an
// agent is allowed to branch on.
func (c liveCLI) wantErrorCode(t *testing.T, code errs.Code, args ...string) envelope {
	t.Helper()
	stdout, stderr, exit := c.run(t, args...)
	var env envelope
	if err := json.Unmarshal([]byte(strings.TrimSpace(stdout)), &env); err != nil {
		t.Fatalf("%v: stdout is not one JSON document (%v)\nstdout=%q stderr=%q exit=%d",
			args, err, stdout, stderr, exit)
	}
	if env.OK {
		t.Fatalf("%v: want ok=false, got %s", args, stdout)
	}
	if env.Error == nil || errs.Code(env.Error.Code) != code {
		t.Fatalf("%v: error code = %+v, want %s", args, env.Error, code)
	}
	if want := 255; code == errs.RemoteCommandTimeout {
		if exit != 124 {
			t.Errorf("%v: timeout must exit 124, got %d", args, exit)
		}
	} else if exit != want {
		t.Errorf("%v: adapter failure must exit 255, got %d", args, exit)
	}
	return env
}

// TestLiveExec proves foreground execution end to end through the CLI: stdout,
// the remote exit status mirrored into the process status, and explicit cwd.
func TestLiveExec(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)

	env := c.mustJSON(t, "--json", "exec", host, "--timeout", "60s", "--command", "echo live-ok; exit 4")
	if got := env.num(t, "exit_code"); got != 4 {
		t.Errorf("data.exit_code = %d, want 4", got)
	}
	if got := strings.TrimSpace(env.str(t, "stdout")); got != "live-ok" {
		t.Errorf("data.stdout = %q, want live-ok", got)
	}

	// The process exit status must mirror the remote command's own status.
	if _, _, exit := c.run(t, "exec", host, "--command", "exit 4"); exit != 4 {
		t.Errorf("process exit = %d, want 4 (remote status must be mirrored)", exit)
	}
	if _, _, exit := c.run(t, "--json", "exec", host, "--command", "true"); exit != 0 {
		t.Errorf("successful exec exit = %d, want 0", exit)
	}

	// --cwd must take effect in the same, stateless context.
	pwd := c.mustJSON(t, "--json", "exec", host, "--cwd", "/var/log", "--command", "pwd")
	if got := strings.TrimSpace(pwd.str(t, "stdout")); got != "/var/log" {
		t.Errorf("cwd /var/log not honoured: stdout=%q", got)
	}

	// exec is stateless by contract: a second process must not see the first's cd.
	again := c.mustJSON(t, "--json", "exec", host, "--command", "pwd")
	if got := strings.TrimSpace(again.str(t, "stdout")); got == "/var/log" {
		t.Errorf("exec leaked cwd between invocations: %q", got)
	}

	// The primary surface keeps one shell string intact and forwards stdin.
	stdout, stderr, exit := c.runInput(t, "streamed input\n", "exec", host,
		"--cwd", "/var/log", "--command", "cat; pwd >&2")
	if stdout != "streamed input\n" || strings.TrimSpace(stderr) != "/var/log" || exit != 0 {
		t.Errorf("direct execution: stdout=%q stderr=%q exit=%d", stdout, stderr, exit)
	}
	direct := c.mustJSON(t, "--json", "exec", host, "--command", "printf direct-json")
	if got := direct.str(t, "stdout"); got != "direct-json" {
		t.Errorf("direct JSON stdout = %q", got)
	}
}

// TestLiveExecTimeout checks the foreground timeout kills the remote process
// group, not just the local ssh, and reports 124 + REMOTE_COMMAND_TIMEOUT.
func TestLiveExecTimeout(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)

	marker := fmt.Sprintf("rhost-livetimeout-%d", time.Now().UnixNano())

	// A distinctive duration, so the probe below cannot match some unrelated sleep.
	sleepFor := strconv.Itoa(300 + time.Now().Nanosecond()%400)

	env := c.wantErrorCode(t, errs.RemoteCommandTimeout,
		"--json", "exec", host, "--timeout", "3s", "--command", "echo "+marker+"; sleep "+sleepFor)
	if !env.bool(t, "timed_out") {
		t.Errorf("data.timed_out = false, want true")
	}

	time.Sleep(500 * time.Millisecond)

	// Match the survivor locally, from a remote command line that contains no
	// pattern of its own. `pgrep -f <marker>` looks equivalent but always matches:
	// rhost passes the whole wrapper script (marker included) as the remote argv,
	// so pgrep's own wrapper is a hit. That false positive once looked like a
	// product bug in the process-group kill; it was not.
	procs := c.mustJSON(t, "--json", "exec", host, "--command", "ps -eo pid,pgid,args").str(t, "stdout")
	if strings.Contains(procs, "sleep "+sleepFor) {
		t.Errorf("remote process group survived the timeout: 'sleep %s' is still running\n%s",
			sleepFor, strings.Join(matchingLines(procs, "sleep "+sleepFor), "\n"))
	}
}

func TestLiveExecDirectCancellation(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	// Keep interruption testing away from the suite-wide master: this test sends
	// SIGINT to its rhost process and should not perturb unrelated parallel work.
	c := cliIsolated(t)
	sleepFor := strconv.Itoa(800 + time.Now().Nanosecond()%100)
	cmd := exec.Command(c.bin, "--json", "exec", host, "--command", "sleep "+sleepFor)
	cmd.Env = append(os.Environ(), c.env...)
	cmd.Dir = c.dir
	var stdout bytes.Buffer
	cmd.Stdout = &stdout
	started := time.Now()
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	// Leave enough time for a cold SSH connection and the remote wrapper to
	// publish its pid. Cancelling before that point is the separate, honest
	// cleanup_confirmed=false case: there may have been nothing remote to kill.
	time.Sleep(6 * time.Second)
	if err := cmd.Process.Signal(os.Interrupt); err != nil {
		t.Fatal(err)
	}
	err := cmd.Wait()
	recordLiveCall(time.Since(started))
	var ee *exec.ExitError
	if !errors.As(err, &ee) || ee.ExitCode() != 130 {
		t.Fatalf("cancel exit = %v, want 130; stdout=%s", err, stdout.String())
	}
	var env envelope
	if err := json.Unmarshal(bytes.TrimSpace(stdout.Bytes()), &env); err != nil {
		t.Fatalf("cancel output is not JSON: %v: %q", err, stdout.String())
	}
	if env.Error == nil || env.Error.Code != string(errs.RemoteCommandCancelled) ||
		!env.bool(t, "cancelled") || !env.bool(t, "cleanup_confirmed") {
		t.Fatalf("cancel result did not confirm cleanup: %s", stdout.String())
	}
	time.Sleep(500 * time.Millisecond)
	procs := c.mustJSON(t, "--json", "exec", host, "--command", "ps -eo args").str(t, "stdout")
	if strings.Contains(procs, "sleep "+sleepFor) {
		t.Fatalf("cancelled remote process survived: sleep %s", sleepFor)
	}
}

func matchingLines(text, needle string) []string {
	var out []string
	for _, line := range strings.Split(text, "\n") {
		if strings.Contains(line, needle) {
			out = append(out, line)
		}
	}
	return out
}

// TestLiveDoctor probes a real host's capabilities.
func TestLiveDoctor(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cli(t)

	env := c.mustJSON(t, "--json", "doctor", host, "--timeout", "60s")
	if got := env.str(t, "os"); got == "" {
		t.Error("doctor returned an empty os")
	}
	var caps map[string]bool
	env.field(t, "capabilities", &caps)
	if !caps["bash"] {
		t.Errorf("capabilities missing bash: %v", caps)
	}
	if !caps["tmux"] {
		t.Log("remote has no tmux: session suite will fail, not skipped")
	}
}

// TestLiveTransportReuse is docs/architecture/engineering.md: two *separate*
// rhost processes must reuse one OpenSSH ControlMaster, and the master must
// outlive the process that created it.
func TestLiveTransportReuse(t *testing.T) {
	t.Parallel()
	host := liveHost(t)
	c := cliIsolated(t)

	c.mustJSON(t, "--json", "exec", host, "--command", "true") // process 1, cold connect

	pid1 := c.masterPID(t, host)
	if pid1 <= 0 {
		t.Fatalf("no ControlMaster running after process 1 exited (got %d): reuse is impossible", pid1)
	}

	c.mustJSON(t, "--json", "exec", host, "--command", "true") // process 2, must reuse

	if pid2 := c.masterPID(t, host); pid2 != pid1 {
		t.Errorf("process 2 did not reuse the transport: master pid %d -> %d", pid1, pid2)
	}

	// The socket must live in rhost's own namespace, never in a shared dir.
	sockets, _ := filepath.Glob(filepath.Join(filepath.Dir(c.controlPath), "*"))
	if len(sockets) != 1 {
		t.Errorf("want exactly 1 rhost control socket in the private namespace, got %v", sockets)
	}
}

// TestLiveControlPathDeepCacheDir proves a long RHOST_CACHE_DIR no longer breaks
// every command: rhost falls back to a short socket root on its own. Without the
// fallback this fails with OpenSSH's "ControlPath too long".
func TestLiveControlPathDeepCacheDir(t *testing.T) {
	host := liveHost(t)
	deep := withCacheDir(t, liveCLI{bin: buildBinary(t)}, deepCacheRoot(t))

	deep.mustJSON(t, "--json", "exec", host, "--command", "echo deep-cache-ok")
	if pid := deep.masterPID(t, host); pid <= 0 {
		t.Errorf("no reusable master on the fallback socket root (pid=%d)", pid)
	}
}

var masterPIDRe = regexp.MustCompile(`Master running \(pid=(\d+)\)`)

// masterPID asks OpenSSH itself whether a multiplexed connection to host is
// alive, using the same ControlPath template rhost passes. A pid means the
// transport outlived the CLI process that opened it.
func (c liveCLI) masterPID(t *testing.T, host string) int {
	t.Helper()
	cmd := exec.Command("ssh",
		"-o", "ControlPath="+c.controlPath,
		"-o", "BatchMode=yes",
		"-O", "check", host)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return -1
	}
	m := masterPIDRe.FindSubmatch(out)
	if m == nil {
		return -1
	}
	pid, err := strconv.Atoi(string(m[1]))
	if err != nil {
		return -1
	}
	return pid
}
