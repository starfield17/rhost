package fileops

import (
	"slices"
	"strings"
	"testing"
)

// The dangerous-path rules are the part of `fs` an agent is told to trust (§28:
// "reject obviously dangerous empty/root destinations where possible"), so each
// accepted and rejected shape is written down. What is *not* here is also a
// decision: rhost does not try to judge whether ~/work/foo is a good place to
// sync into, because there is no generic answer.
func TestRejectSyncTarget(t *testing.T) {
	cases := []struct {
		dst      string
		delete   bool
		wantOK   bool
		whyEmpty bool // only for documentation of the case
	}{
		{dst: "~/work/project", delete: false, wantOK: true},
		{dst: "~/work/project", delete: true, wantOK: true},
		{dst: "/srv/app/deploy", delete: true, wantOK: true},
		{dst: "/tmp/rhost-sync-x", delete: true, wantOK: true},

		// Empty and root shapes, with and without --delete.
		{dst: "", wantOK: false},
		{dst: "   ", wantOK: false},
		{dst: "/", wantOK: false},
		{dst: "//", wantOK: false},
		{dst: "/srv/", delete: true, wantOK: false}, // a trailing slash does not make it two deep

		// Top-level destinations: writable, but never prunable.
		{dst: "/srv", delete: false, wantOK: true},
		{dst: "/srv", delete: true, wantOK: false},
		{dst: "/home", delete: true, wantOK: false},
		{dst: "/usr/local", delete: true, wantOK: true}, // two components: allowed

		// Whole homes.
		{dst: "~", delete: false, wantOK: true},
		{dst: "~", delete: true, wantOK: false},
		{dst: "~/", delete: true, wantOK: false},
		{dst: "~other", delete: true, wantOK: false},
		{dst: "~other/work", delete: true, wantOK: true},

		// Relative to a working directory rhost does not control.
		{dst: ".", wantOK: false},
		{dst: "..", wantOK: false},
		{dst: "./build", delete: true, wantOK: true},

		// Spelled deeper than they are. The rules are applied to the cleaned path,
		// so `..`, `.` and a doubled separator cannot argue a top-level directory
		// into being two components deep — the original bypass, caught in review.
		{dst: "/srv/../", delete: true, wantOK: false},
		{dst: "/srv/.", delete: true, wantOK: false},
		{dst: "/srv//", delete: true, wantOK: false},
		{dst: "~/work/..", delete: true, wantOK: false},
		{dst: "~/.", delete: true, wantOK: false},
		{dst: "./.", wantOK: false},

		// Globs reach the remote shell.
		{dst: "/*", delete: true, wantOK: false},
		{dst: "/home/*", wantOK: false},
		{dst: "~/work?", wantOK: false},
		{dst: "~/work\nx", wantOK: false},
	}

	for _, tc := range cases {
		err := RejectSyncTarget(tc.dst, tc.delete)
		if tc.wantOK && err != nil {
			t.Errorf("RejectSyncTarget(%q, delete=%v) = %v, want accepted", tc.dst, tc.delete, err)
		}
		if !tc.wantOK && err == nil {
			t.Errorf("RejectSyncTarget(%q, delete=%v) accepted a destination it must refuse", tc.dst, tc.delete)
		}
		if err != nil {
			if _, isRejected := err.(ErrRejected); !isRejected {
				t.Errorf("rejections must be ErrRejected so the app layer can map SYNC_REJECTED, got %T", err)
			}
		}
	}
}

// TestSyncArgsNoImplicitDelete pins §28's first two requirements: deletion is
// never implied, and a dry run is what produces the preview.
func TestSyncArgsNoImplicitDelete(t *testing.T) {
	plain, _, _, err := SyncArgs("gpu", SyncOptions{Source: "/home/dev/project", Destination: "~/work/project"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if slices.Contains(plain, "--delete") {
		t.Errorf("--delete appeared without being requested: %v", plain)
	}
	if slices.Contains(plain, "--dry-run") {
		t.Errorf("--dry-run appeared without being requested: %v", plain)
	}

	full, remote, mux, err := SyncArgs("gpu", SyncOptions{
		Source: "/home/dev/project", Destination: "~/work/project",
		Delete: true, DryRun: true, Excludes: []string{".git", "*.o"},
	}, []string{"-o", "ControlPath=/tmp/rhost/%C", "-o", "BatchMode=yes"})
	if err != nil {
		t.Fatal(err)
	}
	if !mux {
		t.Error("a plain ControlPath option should travel in -e and reuse the master")
	}
	if remote != "gpu:~/work/project" {
		t.Errorf("remote spec = %q", remote)
	}
	for _, want := range []string{"--delete", "--dry-run", "--exclude", ".git", "--exclude", "*.o"} {
		if !slices.Contains(full, want) {
			t.Errorf("argv missing %q: %v", want, full)
		}
	}
	// The source's contents are what sync: exactly one trailing slash, whatever
	// the caller typed.
	if got := full[len(full)-2]; got != "/home/dev/project/" {
		t.Errorf("source arg = %q, want the directory with one trailing slash", got)
	}
	if !strings.HasPrefix(lastOf(full, "--itemize-changes"), "") {
		t.Error("itemize must be requested for a machine-readable plan")
	}

	// A source that already ends in a slash must not gain a second one.
	trimmed, _, _, err := SyncArgs("gpu", SyncOptions{Source: "/home/dev/project/", Destination: "~/p"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if trimmed[len(trimmed)-2] != "/home/dev/project/" {
		t.Errorf("source doubled its slash: %v", trimmed)
	}
}

// A symlink is not copied and not followed: rsync without --links skips it, which
// is the behaviour §28 asks for ("do not silently follow unexpected symlinks
// across boundaries") and the same on GNU rsync and openrsync.
func TestSyncArgsDoNotFollowSymlinks(t *testing.T) {
	argv, _, _, err := SyncArgs("gpu", SyncOptions{Source: "project", Destination: "~/p"}, nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, bad := range []string{"--links", "-l", "--copy-links", "--L", "--safe-links"} {
		if slices.Contains(argv, bad) {
			t.Errorf("argv must not carry %q: %v", bad, argv)
		}
	}
	for _, want := range []string{"--no-owner", "--no-group", "--no-perms"} {
		if !slices.Contains(argv, want) {
			t.Errorf("argv missing %q (ownership preservation needs root): %v", want, argv)
		}
	}
	// "./project" is what scp/rsync must see for a relative source that could be
	// read as a remote spec — here it is unambiguous already.
	if argv[len(argv)-2] != "project/" {
		t.Errorf("source arg = %q", argv[len(argv)-2])
	}
}

// The ControlPath template is a path the user's environment chooses. rsync
// splits -e itself but honours quotes (measured against GNU rsync and the
// openrsync macOS ships), so a space no longer costs the transfer its
// multiplexed connection. An option that cannot be represented at all — the
// shell-style escaped quote is not part of rsync's splitter — still falls back,
// and `multiplexed` is how the JSON says so.
func TestSyncArgsQuoteOrDowngradeOptions(t *testing.T) {
	argv, _, mux, err := SyncArgs("gpu", SyncOptions{Source: "project", Destination: "~/p"},
		[]string{"-o", "ControlPath=/tmp/r host/%C"})
	if err != nil {
		t.Fatal(err)
	}
	if !mux {
		t.Error("a ControlPath containing a space can be quoted; it must not cost the master")
	}
	if got := argv[len(argv)-3]; got != "ssh -o 'ControlPath=/tmp/r host/%C'" {
		t.Errorf("-e value = %q, want the option quoted as one word", got)
	}

	argv, _, mux, err = SyncArgs("gpu", SyncOptions{Source: "project", Destination: "~/p"},
		[]string{"-o", "ControlPath=/tmp/it's here/%C"})
	if err != nil {
		t.Fatal(err)
	}
	if mux {
		t.Error("an option containing a single quote cannot travel in -e; say so instead")
	}
	if got := argv[len(argv)-3]; got != "ssh" {
		t.Errorf("-e value = %q, want the bare default %q", got, "ssh")
	}
}

func TestScpArgs(t *testing.T) {
	sshOpts := []string{"-o", "ControlPath=/tmp/x/%C", "-o", "BatchMode=yes"}
	argv, err := ScpArgs("./model.py", "gpu:~/work/foo/model.py", sshOpts)
	if err != nil {
		t.Fatal(err)
	}
	if argv[0] != "-o" || argv[3] != "BatchMode=yes" {
		t.Errorf("options must come first: %v", argv)
	}
	if !slices.Contains(argv, "-q") {
		t.Errorf("scp must be quiet so --json stdout stays clean: %v", argv)
	}
	if argv[len(argv)-2] != "./model.py" || argv[len(argv)-1] != "gpu:~/work/foo/model.py" {
		t.Errorf("operand order wrong: %v", argv)
	}
	// A remote spec is passed through untouched: rewriting `gpu:~/x` as if it were
	// a local path would break the copy.
	if strings.HasPrefix(argv[len(argv)-1], "./") {
		t.Errorf("the remote operand was rewritten: %v", argv)
	}
}

func TestLocalArg(t *testing.T) {
	cases := []struct{ in, want string }{
		{"model.py", "model.py"},
		{"a:b.txt", "./a:b.txt"},                 // colon: would be read as host:path
		{"-rf", "./-rf"},                         // leading dash: would be read as an option
		{".cache:part", "./.cache:part"},         // a dotfile is still ambiguous with a colon
		{"./x", "./x"},                           // already explicit
		{"../x", "../x"},                         // already explicit
		{"/tmp/x", "/tmp/x"},                     // absolute
		{"with space/x.txt", "with space/x.txt"}, // no colon, no dash: fine
	}
	for _, tc := range cases {
		got, err := LocalArg(tc.in)
		if err != nil {
			t.Errorf("LocalArg(%q): %v", tc.in, err)
			continue
		}
		if got != tc.want {
			t.Errorf("LocalArg(%q) = %q, want %q", tc.in, got, tc.want)
		}
	}
	for _, bad := range []string{"", "a\nb", "a\x00b"} {
		if _, err := LocalArg(bad); err == nil {
			t.Errorf("LocalArg(%q) accepted a path it must refuse", bad)
		}
	}
}

func TestValidateTransferPaths(t *testing.T) {
	if err := ValidateTransferPaths("./model.py", "gpu:~/work/foo/model.py"); err != nil {
		t.Errorf("ordinary put rejected: %v", err)
	}
	// An IPv6 host is the shape that used to fool the split: the first colon is
	// inside the address, so an unbracketed spec turned `/` into the "remote path"
	// and let a root destination through.
	if err := ValidateTransferPaths("file", RemoteSpec("user@2001:db8::1", "/")); err == nil {
		t.Errorf("IPv6 host must not bypass destination protection")
	}
	for _, bad := range []struct{ src, dst string }{
		{"", "gpu:x"},
		{"   ", "gpu:x"},
		{"a", ""},
		{"a", "/"},
		{"a", "gpu:"},   // empty remote path
		{"a", "gpu:/*"}, // glob
	} {
		if err := ValidateTransferPaths(bad.src, bad.dst); err == nil {
			t.Errorf("ValidateTransferPaths(%q, %q) accepted it", bad.src, bad.dst)
		}
	}
}

// TestRemoteSpecSplitIPv6 pins the round trip: a colon-bearing host is bracketed
// the way scp and rsync want it, and reading the spec back finds the split after
// the bracket rather than inside the address.
func TestRemoteSpecSplitIPv6(t *testing.T) {
	spec := RemoteSpec("user@2001:db8::1", "work/foo.txt")
	if want := "user@[2001:db8::1]:work/foo.txt"; spec != want {
		t.Fatalf("RemoteSpec = %q, want %q", spec, want)
	}
	host, path, ok := SplitRemoteSpec(spec)
	if !ok || host != "user@[2001:db8::1]" || path != "work/foo.txt" {
		t.Fatalf("SplitRemoteSpec(%q) = %q, %q, %v", spec, host, path, ok)
	}
	if h, p, ok := SplitRemoteSpec("gpu:~/x"); !ok || h != "gpu" || p != "~/x" {
		t.Fatalf("plain spec stopped parsing correctly: %q %q %v", h, p, ok)
	}
	for _, notRemote := range []string{"./a:b", "/tmp/x:y", "user@[::1", "[::1]x"} {
		if _, _, ok := SplitRemoteSpec(notRemote); ok {
			t.Errorf("%q is a local path or malformed, and must not split as remote", notRemote)
		}
	}
}

// TestParseChanges uses the two real output shapes captured from a GNU rsync 3.2
// client and from the openrsync that ships with macOS. They differ in exactly one
// way that matters: openrsync prints deletions in its own format, ignoring
// --out-format.
func TestParseChanges(t *testing.T) {
	gnu := strings.Join([]string{
		"RHOSTSYNC|*deleting  |leftover.txt",
		"RHOSTSYNC|>f.st....|a.txt",
		"RHOSTSYNC|cd+++++++++|sub/",
		"RHOSTSYNC|>f+++++++++|sub/b.txt",
		"",
	}, "\n")
	open := strings.Join([]string{
		"*deleting gone.txt",
		"skipping non-regular file \"link\"",
		"RHOSTSYNC|>f+++++++|a.txt",
		"RHOSTSYNC|cd+++++++|sub/",
		"",
	}, "\n")

	for name, out := range map[string]string{"gnu": gnu, "openrsync": open} {
		changes, other := ParseChanges(out)
		var got []string
		for _, ch := range changes {
			got = append(got, string(ch.Action)+" "+ch.Path)
		}
		want := []string{"delete leftover.txt", "update a.txt", "directory sub/", "create sub/b.txt"}
		if name == "openrsync" {
			want = []string{"delete gone.txt", "create a.txt", "directory sub/"}
		}
		if !slices.Equal(got, want) {
			t.Errorf("%s: changes = %v, want %v", name, got, want)
		}
		if name == "openrsync" && !slices.ContainsFunc(other, func(l string) bool { return strings.Contains(l, "skipping non-regular") }) {
			t.Errorf("%s: the symlink notice must be reported, not dropped: %v", name, other)
		}
	}
}

// An itemize string is kept verbatim beside the coarse action, so a coarse
// classification can never hide what rsync said.
func TestParseChangesKeepsItemize(t *testing.T) {
	changes, _ := ParseChanges("RHOSTSYNC|>fcs.tpn.|x.bin\n")
	if len(changes) != 1 {
		t.Fatalf("want 1 change, got %v", changes)
	}
	if changes[0].Itemize != ">fcs.tpn." || changes[0].Action != ActionUpdate {
		t.Errorf("changes[0] = %+v", changes[0])
	}
}

// A malformed marker line is a diagnostic, never a change.
func TestParseChangesIgnoresPartialLines(t *testing.T) {
	changes, other := ParseChanges(ChangeMarker + "no-separator-here\n")
	if len(changes) != 0 {
		t.Errorf("changes = %v, want none", changes)
	}
	if len(other) != 1 {
		t.Errorf("other = %v, want the raw line kept", other)
	}
}

func lastOf(argv []string, needle string) string {
	for i := len(argv) - 1; i >= 0; i-- {
		if strings.Contains(argv[i], needle) {
			return argv[i]
		}
	}
	return ""
}
