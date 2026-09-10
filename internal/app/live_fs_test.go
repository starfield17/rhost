package app

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/errs"
)

// TestLiveFs is docs/ARCHITECTURE.md §46 M4 against a real host: a put/get
// round-trip, an rsync plan that parses, --delete that prunes only when it was
// asked to, and the dangerous-destination refusal. As in the other live suites,
// every step is a separate process, so what is proven is the CLI contract.

type changeRow struct {
	Action  string `json:"action"`
	Path    string `json:"path"`
	Itemize string `json:"itemize"`
}

func changesOf(t *testing.T, env envelope) []changeRow {
	t.Helper()
	var rows []changeRow
	env.field(t, "changes", &rows)
	return rows
}

func changeMap(rows []changeRow) map[string]string {
	out := map[string]string{}
	for _, r := range rows {
		out[r.Path] = r.Action
	}
	return out
}

// remoteTemp makes a scratch directory on the remote host and removes it again.
func remoteTemp(t *testing.T, c liveCLI, host string) string {
	t.Helper()
	env := c.mustJSON(t, "--json", "exec", host, "--", "mktemp -d")
	dir := strings.TrimSpace(env.str(t, "stdout"))
	if !strings.HasPrefix(dir, "/") {
		t.Fatalf("mktemp -d returned %q", dir)
	}
	t.Cleanup(func() {
		c.run(t, "--json", "exec", host, "--", "rm", "-rf", dir)
	})
	return dir
}

func remoteStdout(t *testing.T, c liveCLI, host, command string) string {
	t.Helper()
	return c.mustJSON(t, "--json", "exec", host, "--", command).str(t, "stdout")
}

func remoteHas(t *testing.T, c liveCLI, host, path string) bool {
	t.Helper()
	return strings.Contains(remoteStdout(t, c, host, "test -e "+path+" && echo yes || echo no"), "yes")
}

// localTree writes a predictable tree: two files, a nested pair, and one file no
// sync should ever copy.
func localTree(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	files := map[string]string{
		"a.txt":      "alpha\n",
		"skip.me":    "do not copy me\n",
		"second.txt": "second\n",
		"sub/b.txt":  "beta\n",
		"sub/deep/c": strings.Repeat("x", 5000),
	}
	for name, body := range files {
		full := filepath.Join(dir, name)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(full, []byte(body), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

func TestLiveFsPutGet(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	local := filepath.Join(t.TempDir(), "model.py")
	body := "# written by the live fs test\nprint('hello')\n"
	if err := os.WriteFile(local, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	remoteDir := remoteTemp(t, c, host)
	remote := remoteDir + "/model.py"

	put := c.mustJSON(t, "--json", "fs", "put", host, local, remote)
	if got := put.str(t, "backend"); got != "scp" {
		t.Errorf("put backend = %q, want scp", got)
	}
	if put.num(t, "size") != len(body) {
		t.Errorf("put size = %d, want %d", put.num(t, "size"), len(body))
	}
	if !strings.Contains(put.str(t, "destination"), remoteDir) {
		t.Errorf("put destination = %q", put.str(t, "destination"))
	}
	// The bytes are really there, read back through a different code path than the
	// transfer used.
	if got := remoteStdout(t, c, host, "cat "+remote); got != body {
		t.Errorf("remote content = %q, want %q", got, body)
	}

	// `get` into an existing directory: the JSON names the file it created rather
	// than the directory it was handed.
	download := t.TempDir()
	get := c.mustJSON(t, "--json", "fs", "get", host, remote, download)
	if filepath.Base(get.str(t, "destination")) != "model.py" {
		t.Errorf("get destination = %q, want model.py inside %s", get.str(t, "destination"), download)
	}
	gotBytes, err := os.ReadFile(get.str(t, "destination"))
	if err != nil {
		t.Fatalf("the file the JSON promised cannot be read: %v", err)
	}
	if !bytes.Equal(gotBytes, []byte(body)) {
		t.Errorf("the round trip changed the bytes: %q", gotBytes)
	}
}

// A local filename that reads as an scp remote spec is what the `./` prefix exists
// for, and only a real scp can prove it.
func TestLiveFsColonFilename(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "a:b.txt"), []byte("colon\n"), 0o644); err != nil {
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
	remoteDir := remoteTemp(t, c, host)

	c.mustJSON(t, "--json", "fs", "put", host, "a:b.txt", remoteDir+"/a:b.txt")
	if got := remoteStdout(t, c, host, "cat "+remoteDir+"/'a:b.txt'"); got != "colon\n" {
		t.Errorf("remote content = %q, want the colon filename copied", got)
	}
}

func TestLiveFsTransferFailures(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	// A missing local source is caught before anything runs: the tool never
	// starts, so this is an invalid invocation rather than a failed transfer —
	// never an internal error and never a silent success (the unit suite pins the
	// same pre-connection validation in TestFsValidatesBeforeConnecting).
	c.wantErrorCode(t, errs.ConfigInvalid, "--json", "fs", "put", host,
		filepath.Join(t.TempDir(), "nope.txt"), "/tmp/rhost-nope.txt")

	// A destination whose parent does not exist fails the same way instead of
	// creating something surprising.
	good := filepath.Join(t.TempDir(), "yes.txt")
	if err := os.WriteFile(good, []byte("x\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	c.wantErrorCode(t, errs.TransferFailed, "--json", "fs", "put", host, good,
		"/definitely/not/here/yes.txt")

	// A directory is the other command's job, and says so before any transfer.
	c.wantErrorCode(t, errs.ConfigInvalid, "--json", "fs", "put", host, t.TempDir(), "/tmp/x")

	// `get` of a remote directory without --recursive is scp's failure to report.
	c.wantErrorCode(t, errs.TransferFailed, "--json", "fs", "get", host, "/tmp",
		filepath.Join(t.TempDir(), "x"))
}

func TestLiveFsSyncPlanAndApply(t *testing.T) {
	host := liveHost(t)
	c := cli(t)

	src := localTree(t)
	dst := remoteTemp(t, c, host) + "/proj"

	// ---- dry run: a parseable plan, and nothing copied.
	plan := c.mustJSON(t, "--json", "fs", "sync", host, src, dst,
		"--dry-run", "--exclude", "skip.me")
	if !plan.bool(t, "dry_run") {
		t.Error("data.dry_run is not true: a caller cannot tell a preview from a copy")
	}
	rows := changeMap(changesOf(t, plan))
	for _, want := range []string{"a.txt", "second.txt", "sub/", "sub/b.txt", "sub/deep/", "sub/deep/c"} {
		if _, ok := rows[want]; !ok {
			t.Errorf("the plan is missing %q: %v", want, rows)
		}
	}
	if _, ok := rows["skip.me"]; ok {
		t.Errorf("--exclude was ignored: %v", rows)
	}
	if plan.num(t, "deletes") != 0 {
		t.Errorf("a plan without --delete listed deletions: %v", rows)
	}
	if remoteHas(t, c, host, dst+"/a.txt") {
		t.Error("--dry-run copied a file")
	}

	// ---- the real sync.
	done := c.mustJSON(t, "--json", "fs", "sync", host, src, dst, "--exclude", "skip.me")
	if done.bool(t, "dry_run") {
		t.Error("the applying sync reported dry_run=true")
	}
	for _, name := range []string{"a.txt", "second.txt", "sub/b.txt", "sub/deep/c"} {
		if !remoteHas(t, c, host, dst+"/"+name) {
			t.Errorf("after sync the remote is missing %s", name)
		}
	}
	if remoteHas(t, c, host, dst+"/skip.me") {
		t.Error("--exclude did not keep the file out of the sync")
	}

	// ---- an identical second sync changes nothing, and says so.
	again := c.mustJSON(t, "--json", "fs", "sync", host, src, dst, "--exclude", "skip.me")
	if len(changesOf(t, again)) != 0 {
		t.Errorf("a no-op sync still reported changes: %v", changesOf(t, again))
	}

	// ---- --delete is explicit, and a dry run only plans it.
	remoteStdout(t, c, host, "touch "+dst+"/orphan.txt")
	prune := c.mustJSON(t, "--json", "fs", "sync", host, src, dst,
		"--exclude", "skip.me", "--delete", "--dry-run")
	if prune.num(t, "deletes") != 1 {
		t.Errorf("planned deletions = %d, want 1 (%v)", prune.num(t, "deletes"), changesOf(t, prune))
	}
	if remoteHas(t, c, host, dst+"/orphan.txt") == false {
		t.Error("--dry-run --delete removed the file")
	}
	applied := c.mustJSON(t, "--json", "fs", "sync", host, src, dst,
		"--exclude", "skip.me", "--delete")
	if applied.num(t, "deletes") != 1 {
		t.Errorf("deletes = %d, want 1", applied.num(t, "deletes"))
	}
	if remoteHas(t, c, host, dst+"/orphan.txt") {
		t.Error("--delete did not prune the remote-only file")
	}

	// The same command without --delete must leave remote-only files alone.
	remoteStdout(t, c, host, "touch "+dst+"/keepme.txt")
	c.mustJSON(t, "--json", "fs", "sync", host, src, dst, "--exclude", "skip.me")
	if !remoteHas(t, c, host, dst+"/keepme.txt") {
		t.Error("a sync without --delete pruned a remote-only file")
	}
}

// TestLiveFsSyncRejectsDangerousTarget exercises the §28 rule end to end: the
// refusal happens before anything is copied, so the remote home stays untouched.
func TestLiveFsSyncRejectsDangerousTarget(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	src := localTree(t)

	for _, target := range []string{"~", "~/", "/", "/tmp"} {
		env := c.wantErrorCode(t, errs.SyncRejected, "--json", "fs", "sync", host, src, target, "--delete")
		// Each refusal must name its own reason: a whole home, a top-level path,
		// or the filesystem root. The point is that the agent learns *why* the
		// prune was refused, not that one particular word was chosen.
		msg := ""
		if env.Error != nil {
			msg = env.Error.Message
		}
		if !strings.Contains(msg, "delete") && !strings.Contains(msg, "top-level") && !strings.Contains(msg, "root") {
			t.Errorf("refusal of %q should say why: %+v", target, env.Error)
		}
	}
	// A glob destination is refused with or without --delete: it would reach the
	// remote shell.
	c.wantErrorCode(t, errs.SyncRejected, "--json", "fs", "sync", host, src, "/tmp/rhost-*")
	// Nothing from the refused syncs landed in the remote home.
	if out := remoteStdout(t, c, host, "ls ~ | grep -c '^a.txt$\\|^second.txt$\\|^sub$' || true"); strings.TrimSpace(out) != "0" {
		t.Errorf("a refused sync still wrote into the remote home: %q", out)
	}
}
