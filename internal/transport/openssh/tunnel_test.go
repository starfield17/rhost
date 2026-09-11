package openssh

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The two guards `tunnel open` promises are both local decisions: which kinds map
// to which ssh flags, and which bind addresses need to be asked for explicitly.
// Whether the remote sshd then permits the forward is its own answer, and rhost
// only reports it — so that part belongs to the live suite, not here.

func TestForwardFlag(t *testing.T) {
	for kind, want := range map[string]string{"local": "-L", "reverse": "-R", "socks": "-D"} {
		got, err := forwardFlag(kind)
		if err != nil || got != want {
			t.Errorf("forwardFlag(%q) = %q, %v; want %q", kind, got, err, want)
		}
	}
	for _, kind := range []string{"", "dynamic", "LOCAL", "-L"} {
		if got, err := forwardFlag(kind); err == nil {
			t.Errorf("forwardFlag(%q) = %q, want an error", kind, got)
		}
	}
}

func TestForwardSpec(t *testing.T) {
	got, err := forwardSpec("local", "localhost:8080", "localhost:8000")
	if err != nil || got != "localhost:8080:localhost:8000" {
		t.Errorf("local spec = %q, %v", got, err)
	}
	got, err = forwardSpec("reverse", "localhost:9000", "localhost:3000")
	if err != nil || got != "localhost:9000:localhost:3000" {
		t.Errorf("reverse spec = %q, %v", got, err)
	}
	// A SOCKS proxy has no destination; giving it one would be silently ignored.
	got, err = forwardSpec("socks", "localhost:1080", "")
	if err != nil || got != "localhost:1080" {
		t.Errorf("socks spec = %q, %v", got, err)
	}
	for _, tc := range []struct{ kind, listen, destination, why string }{
		{"local", "localhost:8080", "", "missing destination"},
		{"local", "localhost:8080", "justahost", "destination without a port"},
		{"local", "localhost", "localhost:1", "listen without a port"},
		{"local", "localhost:0", "localhost:1", "port 0 cannot be reported back"},
		{"reverse", "localhost:0", "localhost:1", "port 0 on either kind"},
	} {
		if spec, err := forwardSpec(tc.kind, tc.listen, tc.destination); err == nil {
			t.Errorf("%s (%s %s): accepted as %q", tc.why, tc.kind, tc.listen, spec)
		}
	}
}

func TestCheckBindExposure(t *testing.T) {
	for _, local := range []string{"localhost:8080", "127.0.0.1:1", "127.0.0.53:53", "[::1]:8080"} {
		if err := checkBind(local, false); err != nil {
			t.Errorf("%s is loopback and must not need --allow-exposure: %v", local, err)
		}
	}
	for _, exposed := range []string{":8080", "0.0.0.0:8080", "192.0.2.10:8080", "[::]:8080"} {
		if err := checkBind(exposed, false); err == nil {
			t.Errorf("%s must require --allow-exposure", exposed)
		}
		if err := checkBind(exposed, true); err != nil {
			t.Errorf("%s with --allow-exposure must be allowed: %v", exposed, err)
		}
	}
}

// TestTunnelRecordIsNamedAfterItself uses a private state dir: a record whose
// contents disagree with its file name must never be used to signal a process,
// because the ID is what selects the socket.
func TestTunnelRecordIsNamedAfterItself(t *testing.T) {
	t.Setenv("RHOST_STATE_DIR", t.TempDir())
	id := "t_" + strings.Repeat("ab", 16)

	if _, err := readTunnel("not-an-id"); err == nil || !errors.Is(err, ErrNoTunnel) {
		t.Errorf("a malformed id = %v, want ErrNoTunnel", err)
	}
	if _, err := readTunnel(id); err == nil || !errors.Is(err, ErrNoTunnel) {
		t.Errorf("a missing record = %v, want ErrNoTunnel", err)
	}

	dir := tunnelRoot()
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, id+".json"),
		[]byte(`{"id":"t_00000000000000000000000000000000","host":"gpu"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := readTunnel(id); err == nil || !strings.Contains(err.Error(), "describes") {
		t.Errorf("a mismatched record = %v, want it named", err)
	}
	// Anything else in the directory is somebody else's name: it is not a tunnel,
	// and never becomes one because a file happened to be there.
	if err := os.WriteFile(filepath.Join(dir, "notes.json"), []byte(`{}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := readTunnel("notes"); err == nil || !errors.Is(err, ErrNoTunnel) {
		t.Errorf("a foreign file must not read as a tunnel: %v", err)
	}
}
