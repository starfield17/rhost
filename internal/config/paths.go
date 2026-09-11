// Package config resolves local filesystem locations used by rhost.
//
// This is deliberately separate from OpenSSH configuration: OpenSSH remains the
// source of truth for hosts, users, keys and host-key policy. rhost only needs a
// private place for its own ControlMaster sockets (and, later, audit/state).
package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

const appName = "rhost"

// CacheDir is rhost's local cache root. Overridable with RHOST_CACHE_DIR.
func CacheDir() string {
	if v := os.Getenv("RHOST_CACHE_DIR"); v != "" {
		return v
	}
	d, err := os.UserCacheDir()
	if err != nil || d == "" {
		d = os.TempDir()
	}
	return filepath.Join(d, appName)
}

// ControlDir holds the OpenSSH ControlMaster sockets for this user. It is the
// single source of truth for where those sockets live: EnsureControlDir creates
// exactly this directory, and ControlPath is derived from it.
//
// A deep cache root plus OpenSSH's fixed 40-byte %C expansion can overflow
// sun_path, and ssh then fails with "ControlPath too long"; in that case a short
// per-user root is used instead.
func ControlDir() string {
	return controlDirIn(CacheDir())
}

// socketFallbackRoot is a short, fixed root for the fallback socket dir. It
// deliberately ignores os.TempDir(): on macOS that is a deep per-session
// /var/folders path, which is exactly what overflowed sun_path.
const socketFallbackRoot = "/tmp"

// maxSocketPath is the conservative sun_path budget (104 bytes including the
// NUL on BSD-derived systems such as macOS; Linux allows 108).
const maxSocketPath = 104

// hashTokenLen is the length OpenSSH substitutes for %C.
const hashTokenLen = 40

func controlDirIn(root string) string {
	dir := filepath.Join(root, "ssh")
	if socketFits(dir) {
		return dir
	}
	return filepath.Join(socketFallbackRoot, fmt.Sprintf("rhost-%d", os.Getuid()), "ssh")
}

// socketFits reports whether the expanded ControlPath under dir stays inside
// sun_path, counting the 40 characters OpenSSH substitutes for %C plus the NUL.
func socketFits(dir string) bool {
	expanded := filepath.Join(dir, strings.Repeat("0", hashTokenLen))
	return len(expanded)+1 <= maxSocketPath
}

// ControlPath is an OpenSSH ControlPath *template*: one socket per target, under
// ControlDir. %C is expanded by OpenSSH to a hash of (local host, remote host,
// port, user), which gives every target its own short, collision-resistant
// socket name.
func ControlPath() string {
	return filepath.Join(ControlDir(), "%C")
}

// EnsureControlDir creates the socket directory with user-only permissions.
func EnsureControlDir() error {
	return os.MkdirAll(ControlDir(), 0o700)
}

// StateDir is rhost's local state root; the audit log lives under it
// (docs/ARCHITECTURE.md §36). It follows the XDG state convention —
// $XDG_STATE_HOME, else ~/.local/state — and RHOST_STATE_DIR overrides it for
// tests and for a user who wants the trail elsewhere. It is deliberately not the
// config dir: state is data rhost accumulates, not configuration.
func StateDir() string {
	if v := os.Getenv("RHOST_STATE_DIR"); v != "" {
		return v
	}
	if d := os.Getenv("XDG_STATE_HOME"); d != "" {
		return filepath.Join(d, appName)
	}
	if h, err := os.UserHomeDir(); err == nil && h != "" {
		return filepath.Join(h, ".local", "state", appName)
	}
	return filepath.Join(os.TempDir(), appName)
}
