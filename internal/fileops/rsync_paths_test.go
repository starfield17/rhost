package fileops

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestValidateRemotePathRejectsOnlyPairedOuterQuotes(t *testing.T) {
	for _, path := range []string{"'~/work/file'", `"/srv/work/file"`} {
		if err := ValidateRemotePath(path); err == nil {
			t.Errorf("ValidateRemotePath(%q) accepted outer quotes", path)
		}
	}
	for _, path := range []string{"author's-note", `report"draft`, "'leading", `trailing"`} {
		if err := ValidateRemotePath(path); err != nil {
			t.Errorf("ValidateRemotePath(%q) = %v", path, err)
		}
	}
}

// The rule under test is which remote paths need help getting through *two*
// parsers — rsync's own split of `host:path`, then the remote login shell — and
// which route is used for each rsync implementation. Both branches are decided by
// asking the local tool what it is, so the tests stand in for the tool rather than
// trusting whatever happens to be installed.

func quote(s string) string { return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'" }

// stubRsync returns a Runner whose "rsync" prints one of the real version banners.
func stubRsync(t *testing.T, banner string) *Runner {
	t.Helper()
	path := filepath.Join(t.TempDir(), "rsync")
	body := "#!/bin/sh\nprintf '%s\\n' " + quote(banner) + "\n"
	if err := os.WriteFile(path, []byte(body), 0o755); err != nil {
		t.Fatal(err)
	}
	return &Runner{RsyncBin: path}
}

// TestProtectArgsDecision pins the banner parsing on its own, with real output
// from real tools. The stub-based tests below go through a spawned process, and a
// loaded machine can make that probe miss its deadline — which is *safe* (the
// conservative route is taken), but must not be the only thing standing between a
// regression and green tests.
func TestProtectArgsDecision(t *testing.T) {
	cases := []struct {
		stdout string
		want   bool
	}{
		{"rsync  version 3.2.7  protocol version 31", true},
		{"rsync  version 3.4.1  protocol version 32", true},
		{"rsync  version 4.0.0pre1  protocol version 32", true},
		{"rsync  version 2.6.9  protocol version 29", false},
		{"openrsync: protocol version 29\nrsync version 2.6.9 compatible", false},
		// A banner that names no version at all is not evidence of --protect-args.
		{"rsync", false},
		{"", false},
		// The decision belongs to the first line: later lines of a real banner list
		// features, and a word appearing there proves nothing about an older tool
		// that lacks the option.
		{"bogus line\nrsync  version 3.2.7  protocol version 31", false},
	}
	for _, tc := range cases {
		if got := protectArgsDecision(tc.stdout); got != tc.want {
			t.Errorf("protectArgsDecision(%q) = %v, want %v", tc.stdout, got, tc.want)
		}
	}
}

func TestPathNeedsQuoting(t *testing.T) {
	for _, plain := range []string{"/srv/app/deploy", "~/work/project", "./a-b_c.1~2"} {
		if pathNeedsQuoting(plain) {
			t.Errorf("%q is inert in a shell and should not need quoting", plain)
		}
	}
	for _, needy := range []string{"/srv/a b", "/srv/x$y", "/srv/[abc]", "/srv/*.go", "/a:b"} {
		if !pathNeedsQuoting(needy) {
			t.Errorf("%q should need quoting", needy)
		}
	}
}

// TestSafeRsyncPathsLeavesPlainPathsAlone pins the cheap common case: no probe, no
// extra option. The rsync binary does not exist here on purpose — reaching for it
// would fail, and it does not.
func TestSafeRsyncPathsLeavesPlainPathsAlone(t *testing.T) {
	r := &Runner{RsyncBin: filepath.Join(t.TempDir(), "not-rsync")}
	args := []string{"--recursive", "-e", "ssh -o X=1", "./src/", "gpu:/srv/app/deploy"}
	got, err := r.SafeRsyncPaths(context.Background(), args)
	if err != nil {
		t.Fatalf("a plain path must not need the local rsync at all: %v", err)
	}
	if strings.Join(got, " ") != strings.Join(args, " ") {
		t.Fatalf("plain paths must pass through untouched, got %v", got)
	}
}

func TestSafeRsyncPathsUsesProtectArgsOnModernGnuRsync(t *testing.T) {
	for _, banner := range []string{
		"rsync  version 3.2.7  protocol version 31",
		"rsync  version 4.0.0  protocol version 32",
	} {
		r := stubRsync(t, banner)
		got, err := r.SafeRsyncPaths(context.Background(), []string{"--times", "./src/", "gpu:/srv/a b"})
		if err != nil {
			t.Fatalf("%s: %v", banner, err)
		}
		if got[0] != "--protect-args" {
			t.Fatalf("%s: expected --protect-args, got %v", banner, got)
		}
		// With --protect-args the path is passed exactly as written; quoting it
		// here would put literal quotes in the remote filename.
		if got[len(got)-1] != "gpu:/srv/a b" {
			t.Errorf("path must stay verbatim, got %q", got[len(got)-1])
		}
	}
}

func TestSafeRsyncPathsQuotesForToolsWithoutProtectArgs(t *testing.T) {
	for _, banner := range []string{
		"openrsync: protocol version 29",
		"rsync  version 2.6.9  protocol version 29",
	} {
		r := stubRsync(t, banner)
		got, err := r.SafeRsyncPaths(context.Background(), []string{"--times", "./src/", "gpu:/srv/a b"})
		if err != nil {
			t.Fatalf("%s: %v", banner, err)
		}
		if got[0] == "--protect-args" {
			t.Fatalf("%s: must not pass an option the tool does not know", banner)
		}
		if want := "gpu:'/srv/a b'"; got[len(got)-1] != want {
			t.Errorf("%s: want the path quoted for the remote shell (%s), got %q", banner, want, got[len(got)-1])
		}
	}
}

// TestSafeRsyncPathsKeepsHomeShorthandWorking is the reason the quoting is
// shell-level and not literal: `~` has to be expanded by the *remote* shell, and
// single quotes would freeze it into a directory named "~".
func TestSafeRsyncPathsKeepsHomeShorthandWorking(t *testing.T) {
	r := stubRsync(t, "openrsync: protocol version 29")
	got, err := r.SafeRsyncPaths(context.Background(), []string{"--times", "./src/", "gpu:~/a b"})
	if err != nil {
		t.Fatal(err)
	}
	if want := `gpu:"$HOME"/'a b'`; got[len(got)-1] != want {
		t.Fatalf("want %s, got %q", want, got[len(got)-1])
	}
}

// TestSafeRsyncPathsUnknownToolFailsConservatively: a tool that answers with
// nothing recognisable is treated as the older kind, because passing an option it
// may not have would break a transfer that could otherwise have run.
func TestSafeRsyncPathsUnknownToolFailsConservatively(t *testing.T) {
	r := stubRsync(t, "rsync -- version ?")
	got, err := r.SafeRsyncPaths(context.Background(), []string{"--times", "./src/", "gpu:/srv/a b"})
	if err != nil {
		t.Fatal(err)
	}
	if got[0] == "--protect-args" {
		t.Fatalf("an unknown banner must not earn --protect-args: %v", got)
	}
}
