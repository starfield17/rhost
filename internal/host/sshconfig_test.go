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

	got, err := Aliases()
	if err != nil {
		t.Fatal(err)
	}
	var names []string
	for _, h := range got {
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
}

func TestAliasesMissingConfigIsNotError(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	got, err := Aliases()
	if err != nil {
		t.Fatalf("Aliases() error = %v, want nil", err)
	}
	if len(got) != 0 {
		t.Fatalf("got %v, want empty", got)
	}
	// `hosts --json` must render [] rather than null: agents iterate the field.
	if got == nil {
		t.Fatal("Aliases returned a nil slice, want an empty non-nil slice")
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
