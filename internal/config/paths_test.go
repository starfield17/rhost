package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCacheDirOverride(t *testing.T) {
	t.Setenv("RHOST_CACHE_DIR", "/tmp/rhost-test-cache")
	if got := CacheDir(); got != "/tmp/rhost-test-cache" {
		t.Errorf("CacheDir() = %q, want override", got)
	}
	if got := ControlDir(); got != filepath.Join("/tmp/rhost-test-cache", "ssh") {
		t.Errorf("ControlDir() = %q", got)
	}
}

func TestControlPathUsesC(t *testing.T) {
	t.Setenv("RHOST_CACHE_DIR", "/tmp/rhost-test-cache")
	got := ControlPath()
	if filepath.Base(got) != "%C" {
		t.Errorf("ControlPath() = %q, want %%C basename", got)
	}
}

func TestControlPathForCacheDoesNotNeedEnvironmentMutation(t *testing.T) {
	root := filepath.Join("/tmp", "rhost-explicit-cache")
	if got := ControlPathForCache(root); got != filepath.Join(root, "ssh", "%C") {
		t.Errorf("ControlPathForCache() = %q", got)
	}
}

func TestEnsureControlDir(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("RHOST_CACHE_DIR", dir)
	if err := EnsureControlDir(); err != nil {
		t.Fatal(err)
	}
	// Assert on ControlDir(), not on a hand-built path: a deep temp root makes
	// rhost resolve the short fallback socket dir instead.
	info, err := os.Stat(ControlDir())
	if err != nil {
		t.Fatalf("control dir not created: %v", err)
	}
	if !info.IsDir() {
		t.Fatal("control dir is not a directory")
	}
	// This directory holds live multiplexed connections for this user.
	if info.Mode().Perm() != 0o700 {
		t.Errorf("control dir mode = %o, want 700", info.Mode().Perm())
	}
	if filepath.Base(ControlDir()) != "ssh" {
		t.Errorf("control dir = %q, want it to end in ssh", ControlDir())
	}
}

func TestEnsurePrivateDirRejectsSymlinkAndTightensMode(t *testing.T) {
	root := t.TempDir()
	dir := filepath.Join(root, "state")
	if err := os.Mkdir(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := ensurePrivateDir(dir); err != nil {
		t.Fatal(err)
	}
	if info, err := os.Stat(dir); err != nil || info.Mode().Perm() != 0o700 {
		t.Fatalf("private dir mode = %v, %v; want 0700", info, err)
	}

	link := filepath.Join(root, "link")
	if err := os.Symlink(dir, link); err != nil {
		t.Fatal(err)
	}
	if err := ensurePrivateDir(link); err == nil {
		t.Error("a symlink must not be accepted as private state")
	}
}

// TestControlPathFitsSocketPath is a regression caught against a real remote
// host: a deep cache dir plus OpenSSH's fixed 40-byte %C expansion overflows
// sun_path, and ssh then refuses every command with "ControlPath too long".
// rhost must move to a short root instead of failing.
func TestControlPathFitsSocketPath(t *testing.T) {
	deep := filepath.Join("/private", strings.Repeat("very-deep-dir-", 10), "Caches", "rhost")

	got := controlDirIn(deep)
	if !strings.HasPrefix(got, socketFallbackRoot) {
		t.Errorf("deep cache root %q did not fall back to the short root, got %q", deep, got)
	}
	if !socketFits(got) {
		t.Errorf("fallback dir still exceeds the socket budget: %q", got)
	}

	shallow := filepath.Join(string(filepath.Separator), "tmp", "rhost-cache")
	if got := controlDirIn(shallow); got != filepath.Join(shallow, "ssh") {
		t.Errorf("a shallow cache root should be used as-is, got %q", got)
	}

	// Whatever the root, the path OpenSSH will actually bind must fit.
	for _, root := range []string{shallow, deep, t.TempDir(), "/tmp"} {
		if !socketFits(controlDirIn(root)) {
			t.Errorf("controlDirIn(%q) = %q exceeds the socket path budget", root, controlDirIn(root))
		}
	}
}

// TestControlDirAvoidsWhitespaceRoot covers the other reason to move the sockets:
// rsync splits its own -e string, and a ControlPath containing a space would have
// to be quoted inside it. The socket path is a value rhost chooses, so it chooses
// one that needs no quoting — and keeps `fs sync` multiplexed.
func TestControlDirAvoidsWhitespaceRoot(t *testing.T) {
	spacey := filepath.Join(string(filepath.Separator), "tmp", "rhost cache with space")
	got := controlDirIn(spacey)
	if !strings.HasPrefix(got, socketFallbackRoot) {
		t.Errorf("a cache root containing whitespace must move the sockets, got %q", got)
	}
	if strings.ContainsAny(got, " \t\n") {
		t.Errorf("the fallback path still contains whitespace: %q", got)
	}
	// The fallback is about the path, not about sync in general: a clean root is
	// still used exactly as given.
	clean := filepath.Join(string(filepath.Separator), "tmp", "rhost-cache")
	if got := controlDirIn(clean); got != filepath.Join(clean, "ssh") {
		t.Errorf("a clean shallow root should be used as-is, got %q", got)
	}
}

// TestControlDirFallbackPreservesCacheIsolation covers cache roots that cannot
// directly hold an OpenSSH socket. Distinct RHOST_CACHE_DIR values still need
// distinct socket namespaces; otherwise one parallel CLI can close or reuse
// another CLI's ControlMaster.
func TestControlDirFallbackPreservesCacheIsolation(t *testing.T) {
	deepA := filepath.Join("/private", strings.Repeat("isolated-cache-a-", 10))
	deepB := filepath.Join("/private", strings.Repeat("isolated-cache-b-", 10))
	spacey := filepath.Join(string(filepath.Separator), "tmp", "isolated cache")

	gotA := controlDirIn(deepA)
	gotB := controlDirIn(deepB)
	gotSpacey := controlDirIn(spacey)

	if gotA == gotB || gotA == gotSpacey || gotB == gotSpacey {
		t.Fatalf("distinct fallback cache roots share a control directory: %q, %q, %q", gotA, gotB, gotSpacey)
	}
	if got := controlDirIn(deepA); got != gotA {
		t.Errorf("fallback directory is not deterministic: first %q, then %q", gotA, got)
	}
	for root, got := range map[string]string{deepA: gotA, deepB: gotB, spacey: gotSpacey} {
		if !strings.HasPrefix(got, socketFallbackRoot+string(filepath.Separator)) {
			t.Errorf("controlDirIn(%q) = %q, want fallback below %q", root, got, socketFallbackRoot)
		}
		if !socketFits(got) {
			t.Errorf("controlDirIn(%q) = %q exceeds the socket path budget", root, got)
		}
		if !rshSafe(got) {
			t.Errorf("controlDirIn(%q) = %q is not safe for rsync -e", root, got)
		}
	}
}

// TestControlPathIsDerivedFromControlDir keeps the two in step: the directory
// rhost creates is the directory it binds in. A mismatch made ssh fail with
// "unix_listener: cannot bind to path ...: No such file or directory".
func TestControlPathIsDerivedFromControlDir(t *testing.T) {
	for _, root := range []string{filepath.Join(t.TempDir(), "ssh"), strings.Repeat("x", 90)} {
		t.Setenv("RHOST_CACHE_DIR", root)
		if got, want := filepath.Dir(ControlPath()), ControlDir(); got != want {
			t.Errorf("ControlPath() dir = %q, want ControlDir() %q", got, want)
		}
		if err := EnsureControlDir(); err != nil {
			t.Fatalf("EnsureControlDir with root %q: %v", root, err)
		}
		if _, err := os.Stat(ControlDir()); err != nil {
			t.Errorf("ControlDir() was not created for root %q: %v", root, err)
		}
	}
}

// TestControlPathHonoursEnvOverrideInChildProcesses documents why live tests set
// RHOST_CACHE_DIR in the environment: the child rhost process and the parent
// must resolve the identical template, with no duplicated path arithmetic.
func TestControlPathHonoursEnvOverrideInChildProcesses(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("RHOST_CACHE_DIR", dir)

	if got, want := ControlPath(), filepath.Join(controlDirIn(dir), "%C"); got != want {
		t.Errorf("ControlPath() = %q, want %q derived from RHOST_CACHE_DIR", got, want)
	}
}

// StateDir anchors the audit log (§36): RHOST_STATE_DIR wins, else the XDG state
// convention, else ~/.local/state — never the config dir.
func TestStateDir(t *testing.T) {
	t.Setenv("RHOST_STATE_DIR", "/tmp/rhost-state")
	if got := StateDir(); got != "/tmp/rhost-state" {
		t.Errorf("StateDir() = %q, want the RHOST_STATE_DIR override", got)
	}

	t.Setenv("RHOST_STATE_DIR", "")
	t.Setenv("XDG_STATE_HOME", "/tmp/xdg-state")
	if got, want := StateDir(), filepath.Join("/tmp/xdg-state", "rhost"); got != want {
		t.Errorf("StateDir() = %q, want %q", got, want)
	}

	t.Setenv("XDG_STATE_HOME", "")
	home := t.TempDir()
	t.Setenv("HOME", home)
	if got, want := StateDir(), filepath.Join(home, ".local", "state", "rhost"); got != want {
		t.Errorf("StateDir() = %q, want %q", got, want)
	}
}
