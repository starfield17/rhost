package app

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/fileops"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// The tests below need no remote host: they pin what `fs` decides *before* and
// *while* it invokes a transfer tool. The stub binary stands in for scp/rsync, so
// the exact argv is observable — which is the only way to test argument
// construction, and argument construction is where a transfer tool does its
// damage.

// stubTool writes argv one per line to log, then behaves like `behaviour`.
func stubTool(t *testing.T, name, behaviour string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), name)
	// The stub records its argv and defines `last`, so a test can talk about the
	// operand the tool would act on without counting options.
	body := "#!/bin/sh\n" +
		": > \"" + path + ".argv\"\n" +
		"last=\"\"; for a in \"$@\"; do printf '%s\\n' \"$a\" >> \"" + path + ".argv\"; last=\"$a\"; done\n" +
		behaviour + "\n"
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
	return path
}

func stubArgv(t *testing.T, stub string) []string {
	t.Helper()
	raw, err := os.ReadFile(stub + ".argv")
	if err != nil {
		t.Fatalf("the tool was never invoked: %v", err)
	}
	return strings.Split(strings.TrimSuffix(string(raw), "\n"), "\n")
}

func newStubApp(scp, rsync string) *App {
	return &App{
		SSH:       openssh.New(openssh.DefaultConfig()),
		Transfers: &fileops.Runner{ScpBin: scp, RsyncBin: rsync},
	}
}

func TestFsPutArgumentShape(t *testing.T) {
	dst := t.TempDir()
	// A colon in the name is the scp-ambiguous shape; an absolute path is the other
	// thing scp's own parser keys on.
	colon := filepath.Join(dst, "a:b.txt")
	if err := os.WriteFile(colon, []byte("hello\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	scp := stubTool(t, "scp", "exit 0")
	a := newStubApp(scp, "rsync")

	res, aerr := a.FsPut(context.Background(), FsPutOptions{
		Host: "gpu", LocalPath: colon, Remote: "/home/dev/work/a:b.txt",
	})
	if aerr != nil {
		t.Fatalf("FsPut: %s: %v", aerr.Code, aerr)
	}
	argv := stubArgv(t, scp)

	// Options first, then the quiet flag, then exactly two operands.
	if argv[0] != "-o" {
		t.Errorf("scp must start with options, got %v", argv)
	}
	var control string
	for _, s := range argv {
		if strings.HasPrefix(s, "ControlPath=") {
			control = s
		}
	}
	if control == "" {
		t.Errorf("scp must carry rhost's own ControlPath so the master is reused: %v", argv)
	}
	if !contains(argv, "BatchMode=yes") {
		t.Errorf("scp must never be able to prompt for a password: %v", argv)
	}
	if !contains(argv, "-q") {
		t.Errorf("scp must be quiet so --json stdout stays clean: %v", argv)
	}
	if argv[len(argv)-1] != "gpu:/home/dev/work/a:b.txt" {
		t.Errorf("remote operand = %q", argv[len(argv)-1])
	}
	if got := argv[len(argv)-2]; got != colon {
		t.Errorf("local operand = %q, want the absolute path unchanged: %q", got, colon)
	}
	// A local path with a colon stays absolute (scp reads an absolute path as
	// local); it is the *relative* shape that needs `./`, pinned in fileops.
	if strings.HasPrefix(argv[len(argv)-2], "./") {
		t.Errorf("an absolute local path must not be rewritten: %v", argv)
	}
	if res.Backend != fileops.BackendScp || res.Size != 6 {
		t.Errorf("result = %+v", res)
	}
	if !res.Multiplexed {
		t.Error("multiplexed must be true when the ControlPath options travelled")
	}
}

// A relative local path whose name contains a colon is the shape that would
// otherwise be read as `host:path`, so the `./` has to reach the tool.
func TestFsPutPrefixesAmbiguousRelativeSource(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "a:b.txt"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	prev, err := os.Getwd()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.Chdir(prev) })
	if err := os.Chdir(dir); err != nil {
		t.Fatal(err)
	}
	scp := stubTool(t, "scp", "exit 0")
	a := newStubApp(scp, "rsync")
	if _, aerr := a.FsPut(context.Background(), FsPutOptions{
		Host: "gpu", LocalPath: "a:b.txt", Remote: "/tmp/a:b.txt",
	}); aerr != nil {
		t.Fatal(aerr)
	}
	argv := stubArgv(t, scp)
	if got := argv[len(argv)-2]; got != "./a:b.txt" {
		t.Errorf("local operand = %q, want ./a:b.txt", got)
	}
}

func TestFsGetReportsTheFileItWrote(t *testing.T) {
	dir := t.TempDir()
	body := "fetched\n"
	// The stub stands in for scp's behaviour of writing into a directory operand.
	scp := stubTool(t, "scp", "printf '"+body+"' > \"$last/model.py\"; exit 0")
	a := newStubApp(scp, "rsync")

	res, aerr := a.FsGet(context.Background(), FsGetOptions{
		Host: "gpu", Remote: "/home/dev/work/model.py", LocalPath: dir,
	})
	if aerr != nil {
		t.Fatalf("FsGet: %v", aerr)
	}
	if got := filepath.Base(res.Destination); got != "model.py" {
		t.Errorf("destination = %q, want the fetched file inside %s", res.Destination, dir)
	}
	if res.Size != int64(len(body)) {
		t.Errorf("size = %d, want %d", res.Size, len(body))
	}
}

func TestFsMapsToolFailures(t *testing.T) {
	src := filepath.Join(t.TempDir(), "f.txt")
	if err := os.WriteFile(src, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}

	scp := stubTool(t, "scp", "printf 'scp: /nope: No such file or directory\\n' >&2; exit 1")
	a := newStubApp(scp, "rsync")
	_, aerr := a.FsPut(context.Background(), FsPutOptions{Host: "gpu", LocalPath: src, Remote: "/nope"})
	if aerr == nil || aerr.Code != errs.TransferFailed {
		t.Fatalf("a failing scp must be TRANSFER_FAILED, got %+v", aerr)
	}
	if !strings.Contains(aerr.Message, "No such file or directory") {
		t.Errorf("the tool's own reason belongs in the message: %q", aerr.Message)
	}
	if !aerr.Retryable {
		t.Error("a transfer failure is reported retryable: rhost does not sort causes by matching text (§33)")
	}

	// A tool that cannot be started at all is not the remote's fault.
	a = newStubApp(filepath.Join(t.TempDir(), "not-installed"), "rsync")
	_, aerr = a.FsPut(context.Background(), FsPutOptions{Host: "gpu", LocalPath: src, Remote: "/tmp/x"})
	if aerr == nil || aerr.Code != errs.TransferFailed ||
		!strings.Contains(aerr.Message, "not installed") {
		t.Fatalf("a missing local scp must say so: %+v", aerr)
	}

	// A transfer that outlives its budget is a timeout, the same code exec uses.
	//
	// The stub explicitly forks a shell that backgrounds the sleep and waits for
	// it, so a child holding the inherited stdout/stderr pipe exists when the tool
	// is killed — the scenario the timeout must handle. A plain `sleep 5; exit 0`
	// only reproduced it on Linux (dash forks the sleep), not on macOS (bash often
	// exec'd it), which is why the failure was CI-only: the timeout left the tool
	// running for the full 5s.
	slow := stubTool(t, "scp", "/bin/sh -c 'sleep 5 & wait'")
	a = newStubApp(slow, "rsync")
	start := time.Now()
	_, aerr = a.FsPut(context.Background(), FsPutOptions{
		Host: "gpu", LocalPath: src, Remote: "/tmp/x", Timeout: 300 * time.Millisecond,
	})
	if aerr == nil || aerr.Code != errs.RemoteCommandTimeout {
		t.Fatalf("a slow transfer must be REMOTE_COMMAND_TIMEOUT, got %+v", aerr)
	}
	if time.Since(start) > 3*time.Second {
		t.Errorf("the timeout did not stop the tool: took %s", time.Since(start))
	}
}

// The validation rules must answer before any network or tool is touched — that
// is what makes `fs sync --delete ~` safe to try.
func TestFsValidatesBeforeConnecting(t *testing.T) {
	dir := t.TempDir()
	a := newStubApp("definitely-not-scp", "definitely-not-rsync")

	_, aerr := a.FsSync(context.Background(), FsSyncOptions{
		Host: "no-such-host.invalid", LocalPath: dir, Remote: "~", Delete: true,
	})
	if aerr == nil || aerr.Code != errs.SyncRejected {
		t.Fatalf("a --delete sync into ~ must be SYNC_REJECTED, got %+v", aerr)
	}

	// A file, not a directory, is the other command's job.
	file := filepath.Join(dir, "f.txt")
	if err := os.WriteFile(file, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	_, aerr = a.FsSync(context.Background(), FsSyncOptions{
		Host: "no-such-host.invalid", LocalPath: file, Remote: "~/work/x",
	})
	if aerr == nil || aerr.Code != errs.ConfigInvalid ||
		!strings.Contains(aerr.Message, "fs put") {
		t.Fatalf("syncing a file must point at fs put: %+v", aerr)
	}

	// put of a directory likewise.
	_, aerr = a.FsPut(context.Background(), FsPutOptions{
		Host: "no-such-host.invalid", LocalPath: dir, Remote: "/tmp/dir",
	})
	if aerr == nil || aerr.Code != errs.ConfigInvalid {
		t.Fatalf("putting a directory must be CONFIG_INVALID, got %+v", aerr)
	}

	// A missing local file is reported as what it is, before connecting.
	_, aerr = a.FsPut(context.Background(), FsPutOptions{
		Host: "no-such-host.invalid", LocalPath: filepath.Join(dir, "gone.txt"), Remote: "/tmp/x",
	})
	if aerr == nil || aerr.Code != errs.ConfigInvalid {
		t.Fatalf("a missing source must be CONFIG_INVALID, got %+v", aerr)
	}
}

// A glob in a destination would be expanded by the remote shell, so it is refused
// as a rejection rather than a transfer failure.
func TestFsRejectsGlobEndpoints(t *testing.T) {
	a := newStubApp("definitely-not-scp", "definitely-not-rsync")
	_, aerr := a.FsSync(context.Background(), FsSyncOptions{
		Host: "no-such-host.invalid", LocalPath: t.TempDir(), Remote: "/tmp/rhost-*",
	})
	if aerr == nil || aerr.Code != errs.SyncRejected {
		t.Fatalf("a glob destination must be SYNC_REJECTED, got %+v", aerr)
	}
}

// pathBase decides the name a `get` into a directory reports.
func TestPathBase(t *testing.T) {
	cases := map[string]string{
		"/home/dev/work/model.py": "model.py",
		"model.py":                "model.py",
		"gpu:/home/dev/x.tar.gz":  "x.tar.gz",
		"~/data/":                 "data",
		"/":                       "rhost-download",
	}
	for in, want := range cases {
		if got := pathBase(in); got != want {
			t.Errorf("pathBase(%q) = %q, want %q", in, got, want)
		}
	}
}

func contains(hay []string, needle string) bool {
	for _, s := range hay {
		if s == needle {
			return true
		}
	}
	return false
}
