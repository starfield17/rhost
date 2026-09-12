package cli

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
)

func TestRunBatchManifestValidation(t *testing.T) {
	write := func(t *testing.T, body string) string {
		t.Helper()
		path := filepath.Join(t.TempDir(), "manifest.json")
		if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	if _, e := readBatchManifest(""); e == nil || e.Code != errs.UsageError {
		t.Errorf("missing --manifest = %v, want USAGE_ERROR", e)
	}
	for _, bad := range []string{
		`{}`, // not an array
		`[]`, // nothing to do
		`[{"op":"rm","source":"a","destination":"b"}]`, // op must be put or get
		`[{"op":"put","source":"","destination":"b"}]`, // an empty endpoint
	} {
		if _, e := readBatchManifest(write(t, bad)); e == nil {
			t.Errorf("manifest %q was accepted", bad)
		}
	}
	entries, e := readBatchManifest(write(t,
		`[{"op":"get","source":"h:r.txt","destination":"./l.txt","resume":true}]`))
	if e != nil {
		t.Fatal(e)
	}
	if len(entries) != 1 || entries[0].Op != "get" || !entries[0].Resume {
		t.Fatalf("manifest did not parse as written: %+v", entries)
	}
}

func TestHelperUsageErrorBounds(t *testing.T) {
	for _, tc := range []struct {
		op                     string
		maxBytes, start, lines int
	}{
		{op: "read", maxBytes: 0},
		{op: "read", maxBytes: 10, start: 0},
		{op: "read", maxBytes: 10, lines: 0},
		{op: "read", maxBytes: maxRemoteHelperBytes + 1, start: 1, lines: 1},
	} {
		if e := helperUsageError(tc.op, tc.maxBytes, tc.start, tc.lines); e == nil {
			t.Errorf("%+v accepted a bound the remote helper would refuse", tc)
		}
	}
	if e := helperUsageError("read", 100, 1, 200); e != nil {
		t.Errorf("valid read rejected: %v", e)
	}
	// write and patch carry no paging bounds at all.
	if e := helperUsageError("write", 100, 0, 0); e != nil {
		t.Errorf("valid write rejected: %v", e)
	}
}

func TestTunnelOpenValidationIsConfigurationError(t *testing.T) {
	for _, tc := range []struct {
		kind, listen, destination string
		expose                    bool
	}{
		{kind: "dynamic", listen: "localhost:1080"},
		{kind: "local", listen: "localhost", destination: "localhost:8000"},
		{kind: "local", listen: "localhost:8080"},
		{kind: "socks", listen: "localhost:1080", destination: "localhost:8000"},
		{kind: "local", listen: ":8080", destination: "localhost:8000"},
	} {
		e := validateTunnelOpen(tc.kind, tc.listen, tc.destination, tc.expose)
		if e == nil || e.Code != errs.ConfigInvalid || e.Retryable {
			t.Errorf("validateTunnelOpen(%+v) = %v, want non-retryable CONFIG_INVALID", tc, e)
		}
	}
}

func TestDoctorHumanCapabilitiesIncludeRemoteHelpers(t *testing.T) {
	t.Cleanup(saveGlobals())
	jsonFlag = false
	res := app.DoctorResult{Host: "example-host", Capabilities: map[string]bool{
		"python3": true, "realpath": true,
	}}
	out := captureStdout(t, func() { renderDoctor(res) })
	for _, name := range []string{"python3", "realpath"} {
		if !strings.Contains(out, name) {
			t.Errorf("doctor human output omitted %s:\n%s", name, out)
		}
	}
}

func TestLoadPatchRequiresHashAndEdits(t *testing.T) {
	// A patch with no hash cannot be checked against anything, so it is refused
	// before it is sent rather than becoming a remote FILE_CONFLICT.
	if e := loadPatch(map[string]interface{}{"if_hash": ""}, []byte(`{"edits":[{"start":1}]}`), "h"); e == nil ||
		e.Code != errs.ConfigInvalid {
		t.Errorf("patch without sha256 accepted: %v", e)
	}
	if e := loadPatch(map[string]interface{}{"if_hash": ""}, []byte(`{"sha256":"abc"}`), "h"); e == nil {
		t.Errorf("patch without edits accepted")
	}
	if e := loadPatch(map[string]interface{}{"if_hash": ""}, []byte(`not json`), "h"); e == nil {
		t.Errorf("patch that is not JSON accepted")
	}

	request := map[string]interface{}{"if_hash": ""}
	if e := loadPatch(request, []byte(`{"sha256":"doc","edits":[{"start":1,"end":1,"text":"x"}]}`), "h"); e != nil {
		t.Fatal(e)
	}
	if request["if_hash"] != "doc" {
		t.Errorf("document hash not used: %v", request["if_hash"])
	}
	request = map[string]interface{}{"if_hash": "cli"}
	if e := loadPatch(request, []byte(`{"sha256":"doc","edits":[{"start":1,"end":1,"text":"x"}]}`), "h"); e != nil {
		t.Fatal(e)
	}
	// --if-hash on the command line wins, which is how a reviewed patch file is
	// applied against a hash read moments ago.
	if request["if_hash"] != "cli" {
		t.Errorf("--if-hash must override the document, got %v", request["if_hash"])
	}
}

func TestRenderFsMirrorUsesDownloadVerbs(t *testing.T) {
	t.Cleanup(saveGlobals())
	jsonFlag = false
	res := app.FsSyncResult{
		Source: "gpu:~/work/project/", Destination: "./download",
		Backend: "rsync", DryRun: true, Delete: true, Multiplexed: true,
	}
	out := captureStdout(t, func() { renderSyncDirection(res, "downloaded", "would download (dry run, nothing copied)") })
	if !strings.Contains(out, "would download") || !strings.Contains(out, "--delete") {
		t.Errorf("mirror preview reads like an upload: %q", out)
	}
	if strings.Contains(out, "would sync") {
		t.Errorf("mirror must not be described as a sync: %q", out)
	}
}
