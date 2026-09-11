package app

import (
	"encoding/json"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/shell"
)

// TestLiveTools walks the file and exec surface through *separate CLI processes*,
// because that is how an agent uses it: each call starts a new process, so what
// survives between them is only what OpenSSH and the remote host keep (AGENTS.md
// §4). It is one test rather than eight so a real round trip is paid once, but
// every step asserts on the `--json` envelope rather than on human text — the
// envelope is the contract (AGENTS.md §6).
//
// What stays unverified here, and is therefore not claimed anywhere: the helper's
// behaviour under a hostile path (covered by its own python suite), and anything
// about a host that reboots mid-operation.
func TestLiveTools(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	dir := remoteTemp(t, c, host)
	local := filepath.Join(t.TempDir(), "input.txt")
	if err := os.WriteFile(local, []byte("alpha\nbeta\n"), 0600); err != nil {
		t.Fatal(err)
	}
	target := dir + "/text.txt"
	write := c.mustJSON(t, "--json", "fs", "write", host, target, "--from", local)
	read := c.mustJSON(t, "--json", "fs", "read", host, target)
	if read.str(t, "content") != "alpha\nbeta\n" || read.str(t, "sha256") != write.str(t, "sha256") {
		t.Fatal("read/write mismatch")
	}
	patch := filepath.Join(t.TempDir(), "patch.json")
	data, _ := json.Marshal(map[string]interface{}{"sha256": read.str(t, "sha256"), "edits": []interface{}{map[string]interface{}{"start": 2, "end": 2, "text": "gamma\n"}}})
	if err := os.WriteFile(patch, data, 0600); err != nil {
		t.Fatal(err)
	}
	c.mustJSON(t, "--json", "fs", "patch", host, target, "--patch", patch)
	if got := c.mustJSON(t, "--json", "fs", "read", host, target).str(t, "content"); got != "alpha\ngamma\n" {
		t.Fatal(got)
	}
	raw, _, code := c.run(t, "--json", "fs", "patch", host, target, "--patch", patch)
	var conflict envelope
	if err := json.Unmarshal([]byte(raw), &conflict); err != nil || code != 255 || conflict.Error == nil || conflict.Error.Code != "FILE_CONFLICT" {
		t.Fatalf("expected conflict: %s", raw)
	}
	// Re-applying the same patch must be *refused*, not applied twice: the hash the
	// patch carries no longer describes the file. This is the whole reason the
	// command group is safe to hand to an agent.
	for _, op := range []string{"grep", "glob"} {
		pattern := "gamma"
		if op == "glob" {
			pattern = "*.txt"
		}
		result := c.mustJSON(t, "--json", "fs", op, host, pattern, dir)
		var rows []interface{}
		result.field(t, "results", &rows)
		if len(rows) != 1 {
			t.Fatalf("%s results: %s", op, result.Data)
		}
	}
	for _, mode := range []string{"files", "count"} {
		result := c.mustJSON(t, "--json", "fs", "grep", host, "gamma", dir, "--mode", mode)
		var rows []interface{}
		result.field(t, "results", &rows)
		if len(rows) != 1 {
			t.Fatalf("search mode %s: %s", mode, result.Data)
		}
	}
	c.mustJSON(t, "--json", "fs", "put", host, local, dir+"/verified.txt", "--checksum")
	c.mustJSON(t, "--json", "fs", "get", host, dir+"/verified.txt", filepath.Join(t.TempDir(), "get.txt"), "--resume")
	c.mustJSON(t, "--json", "fs", "put", host, local, dir+"/file with space.txt", "--checksum")
	manifest := filepath.Join(t.TempDir(), "manifest.json")
	batchDownload := filepath.Join(t.TempDir(), "download.txt")
	manifestData, _ := json.Marshal([]interface{}{map[string]string{"op": "put", "source": local, "destination": dir + "/batch.txt"}, map[string]string{"op": "get", "source": dir + "/batch.txt", "destination": batchDownload}})
	if err := os.WriteFile(manifest, manifestData, 0600); err != nil {
		t.Fatal(err)
	}
	c.mustJSON(t, "--json", "fs", "batch", host, "--manifest", manifest)
	if body, err := os.ReadFile(batchDownload); err != nil || string(body) != "alpha\nbeta\n" {
		t.Fatalf("batch round trip: %s %v", body, err)
	}
	dest := t.TempDir()
	c.mustJSON(t, "--json", "fs", "mirror", host, dir, dest)
	if body, err := os.ReadFile(filepath.Join(dest, "text.txt")); err != nil || string(body) != "alpha\ngamma\n" {
		t.Fatalf("mirror: %s %v", body, err)
	}
	capped := c.mustJSON(t, "--json", "exec", host, "--max-output-bytes", "1000", "--", "head -c 100000 /dev/zero | tr '\\0' x")
	var truncated bool
	capped.field(t, "stdout_truncated", &truncated)
	if len(capped.str(t, "stdout")) != 1000 || !truncated || capped.num(t, "stdout_bytes") != 100000 {
		t.Fatalf("bounded output: %s", capped.Data)
	}
	many := c.mustJSON(t, "--json", "exec-many", "--host", host, "--host", host, "--", "printf batch")
	var rows []map[string]interface{}
	many.field(t, "results", &rows)
	if len(rows) != 2 || rows[0]["stdout"] != "batch" || rows[1]["stdout"] != "batch" {
		t.Fatalf("batch exec: %s", many.Data)
	}
}

// TestLiveTunnelReverseTraffic proves a reverse forward by pushing bytes through
// it from the *remote* side, because `ssh -O check` on a master that is up says
// nothing about whether the port OpenSSH asked the remote sshd for was really
// bound — a refused reverse bind is exactly the failure an agent cannot see from
// the local machine. The service answering is on this side, so the assertion is
// about the tunnel and not about any remote daemon.
func TestLiveTunnelReverseTraffic(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	t.Setenv("RHOST_STATE_DIR", t.TempDir())
	server, err := net.Listen("tcp", "localhost:0")
	if err != nil {
		t.Fatal(err)
	}
	defer server.Close()
	go func() {
		conn, err := server.Accept()
		if err == nil {
			defer conn.Close()
			conn.Write([]byte("rhost-tunnel-ok"))
		}
	}()
	port := strings.TrimSpace(remoteStdout(t, c, host, "python3 -c "+shell.Quote("import socket; s=socket.socket(); s.bind(('localhost',0)); print(s.getsockname()[1]); s.close()")))
	opened := c.mustJSON(t, "--json", "tunnel", "open", host, "--kind", "reverse", "--listen", net.JoinHostPort("localhost", port), "--destination", server.Addr().String())
	id := opened.str(t, "id")
	t.Cleanup(func() { c.run(t, "--json", "tunnel", "close", id) })
	command := "python3 -c " + shell.Quote("import socket; s=socket.create_connection(('localhost',"+port+"),5); print(s.recv(100).decode()); s.close()")
	if got := strings.TrimSpace(remoteStdout(t, c, host, command)); got != "rhost-tunnel-ok" {
		t.Fatalf("reverse tunnel response: %q", got)
	}
}

// TestLiveSessionRecoveryExplicit is the persistence claim of the session group
// (§4): `recover` runs in a process that shares nothing with the one that created
// the session, so if the shell state is still there afterwards it can only be the
// remote tmux's doing.
func TestLiveSessionRecoveryExplicit(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	name := "recover-" + strconv.FormatInt(time.Now().UnixNano(), 36)
	c.mustJSON(t, "--json", "session", "create", host, "--name", name)
	t.Cleanup(func() { c.run(t, "--json", "session", "close", host, name) })
	c.mustJSON(t, "--json", "session", "exec", host, name, "--", "export RHOST_RECOVERY_VALUE=retained")
	result := c.mustJSON(t, "--json", "session", "recover", host, name)
	var preserved bool
	result.field(t, "session_preserved", &preserved)
	if !preserved {
		t.Fatal("recovery did not preserve session")
	}
	if got := c.mustJSON(t, "--json", "session", "exec", host, name, "--", "printf '%s' \"$RHOST_RECOVERY_VALUE\"").str(t, "output"); !strings.Contains(got, "retained") {
		t.Fatalf("lost shell state: %q", got)
	}
}

// TestLiveTunnelPersistence checks the record, not just the socket: a tunnel that
// `list` cannot rediscover is one whose id the caller has no way to keep, and the
// id is the entire interface to closing it.
func TestLiveTunnelPersistence(t *testing.T) {
	host := liveHost(t)
	c := cli(t)
	t.Setenv("RHOST_STATE_DIR", t.TempDir())
	listener, err := net.Listen("tcp", "localhost:0")
	if err != nil {
		t.Fatal(err)
	}
	portNum := listener.Addr().(*net.TCPAddr).Port
	listener.Close()
	opened := c.mustJSON(t, "--json", "tunnel", "open", host, "--kind", "socks",
		"--listen", net.JoinHostPort("localhost", strconv.Itoa(portNum)))
	id := opened.str(t, "id")
	t.Cleanup(func() { c.run(t, "--json", "tunnel", "close", id) })

	type tunnelRow struct {
		ID     string `json:"id"`
		Status string `json:"status"`
		Kind   string `json:"kind"`
		Listen string `json:"listen"`
	}
	lookup := func(rows []tunnelRow, want string) tunnelRow {
		for _, row := range rows {
			if row.ID == want {
				return row
			}
		}
		return tunnelRow{}
	}

	var listed []tunnelRow
	c.mustJSON(t, "--json", "tunnel", "list").field(t, "tunnels", &listed)
	row := lookup(listed, id)
	// "alive" is OpenSSH's answer, not rhost's guess; a record that disagrees with
	// the socket is worse than no record, because the caller keeps an id that names
	// nothing.
	if row.Status != "alive" || row.Kind != "socks" {
		t.Fatalf("rediscovered %s as %+v, want an alive socks forward", id, row)
	}
	if _, port, err := net.SplitHostPort(row.Listen); err != nil || port != strconv.Itoa(portNum) {
		t.Fatalf("rediscovered listen address %q for %s, want port %d", row.Listen, id, portNum)
	}

	// The record says the forward exists; the local socket has to agree, since an
	// unbound listener is exactly what a stale record looks like from here.
	conn, err := net.Dial("tcp", net.JoinHostPort("localhost", strconv.Itoa(portNum)))
	if err != nil {
		t.Fatalf("nothing listening on the forwarded port %d: %v", portNum, err)
	}
	_ = conn.Close()

	c.mustJSON(t, "--json", "tunnel", "close", id)
	// Closing removes the record as well as the master: an id left behind looks
	// like a forward that is still open.
	var after []tunnelRow
	c.mustJSON(t, "--json", "tunnel", "list").field(t, "tunnels", &after)
	if row := lookup(after, id); row.Status != "" {
		t.Fatalf("tunnel %s still listed after close: %+v", id, row)
	}
}
