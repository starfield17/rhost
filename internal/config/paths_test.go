package config

import (
	"os"
	"path/filepath"
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

func TestEnsureControlDir(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("RHOST_CACHE_DIR", dir)
	if err := EnsureControlDir(); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(filepath.Join(dir, "ssh"))
	if err != nil {
		t.Fatalf("control dir not created: %v", err)
	}
	if !info.IsDir() {
		t.Fatal("control dir is not a directory")
	}
}
