// Package config resolves local filesystem locations used by rhost.
//
// This is deliberately separate from OpenSSH configuration: OpenSSH remains the
// source of truth for hosts, users, keys and host-key policy. rhost only needs a
// private place for its own ControlMaster sockets (and, later, audit/state).
package config

import (
	"os"
	"path/filepath"
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

// ControlDir holds the OpenSSH ControlMaster sockets for this user.
func ControlDir() string {
	return filepath.Join(CacheDir(), "ssh")
}

// ControlPath is an OpenSSH ControlPath *template*.
//
// %C is expanded by OpenSSH to a hash of (local host, remote host, port, user),
// which gives every target its own short, collision-resistant socket name and
// keeps us well under the Unix-domain socket path length limit.
func ControlPath() string {
	return filepath.Join(ControlDir(), "%C")
}

// EnsureControlDir creates the socket directory with user-only permissions.
func EnsureControlDir() error {
	return os.MkdirAll(ControlDir(), 0o700)
}

// StateDir is the local state root (audit log, etc.). Not yet used in v0.1 M1.
func StateDir() string {
	if v := os.Getenv("RHOST_STATE_DIR"); v != "" {
		return v
	}
	d, err := os.UserConfigDir()
	if err != nil || d == "" {
		return filepath.Join(os.TempDir(), appName)
	}
	return filepath.Join(d, appName)
}
