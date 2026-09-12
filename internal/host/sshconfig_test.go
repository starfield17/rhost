package host

import (
	"os"
	"path/filepath"
	"testing"
)

func TestAliasesParsesConfigAndIncludes(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	sshDir := filepath.Join(home, ".ssh")
	if err := os.MkdirAll(sshDir, 0o700); err != nil {
		t.Fatal(err)
	}
	main := "Host gpu\n" +
		"    HostName gpu.example.internal\n" +
		"Host = db\n" +
		"    User root\n" +
		"Host *.example.com\n" + // wildcard: skipped
		"Host !blocked\n" + // negated: skipped
		"Include conf.d/*.conf\n"
	if err := os.WriteFile(filepath.Join(sshDir, "config"), []byte(main), 0o600); err != nil {
		t.Fatal(err)
	}
	confDir := filepath.Join(sshDir, "conf.d")
	if err := os.MkdirAll(confDir, 0o700); err != nil {
		t.Fatal(err)
	}
	extra := "Host extra\n    HostName 1.2.3.4\n"
	if err := os.WriteFile(filepath.Join(confDir, "more.conf"), []byte(extra), 0o600); err != nil {
		t.Fatal(err)
	}

	result, err := Aliases()
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, h := range result.Hosts {
		names = append(names, h.Alias)
	}
	want := map[string]bool{"gpu": true, "db": true, "extra": true}
	if len(names) != len(want) {
		t.Fatalf("aliases = %v, want %d entries", names, len(want))
	}
	for _, n := range names {
		if !want[n] {
			t.Errorf("unexpected alias %q", n)
		}
	}
	if !result.ConfigFound || !result.Complete || len(result.Warnings) != 0 {
		t.Fatalf("discovery metadata = %+v", result)
	}
}

func TestAliasesMissingConfigIsNotError(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	result, err := Aliases()
	if err != nil {
		t.Fatalf("Aliases() error = %v, want nil", err)
	}
	if len(result.Hosts) != 0 {
		t.Fatalf("got %v, want empty", result.Hosts)
	}
	// `hosts --json` must render [] rather than null: agents iterate the field.
	if result.Hosts == nil {
		t.Fatal("Aliases returned a nil slice, want an empty non-nil slice")
	}
	if result.ConfigFound || !result.Complete {
		t.Fatalf("missing config metadata = %+v", result)
	}
}

func TestAliasesReportsIncompleteInclude(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	sshDir := filepath.Join(home, ".ssh")
	if err := os.MkdirAll(filepath.Join(sshDir, "unreadable"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(sshDir, "config"), []byte("Include unreadable\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	result, err := Aliases()
	if err != nil {
		t.Fatal(err)
	}
	if result.Complete || len(result.Warnings) == 0 {
		t.Fatalf("incomplete discovery was hidden: %+v", result)
	}
}

func TestSplitKeyword(t *testing.T) {
	cases := []struct{ in, key, rest string }{
		{"Host gpu", "Host", "gpu"},
		{"Host=gpu", "Host", "gpu"},
		{"  HostName   build.example.internal", "HostName", "build.example.internal"},
		{"Include conf.d/*.conf", "Include", "conf.d/*.conf"},
	}
	for _, c := range cases {
		k, r := splitKeyword(c.in)
		if k != c.key || r != c.rest {
			t.Errorf("splitKeyword(%q) = (%q,%q), want (%q,%q)", c.in, k, r, c.key, c.rest)
		}
	}
}
