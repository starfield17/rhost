package cli

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

// These tests cover the decisions the new commands make *locally*: what counts as
// an aggregate failure, what a manifest must contain, how a search row reads to a
// human. None of them needs a remote host, and each one is a rule an agent is told
// to rely on, so the rule is written down twice on purpose — once in the code,
// once as a case here.

func okRow(host string, exit int) execManyResult {
	return execManyResult{Host: host, OK: true, execView: execView{ExitCode: exit}}
}

func failingRow(host string) execManyResult {
	return execManyResult{Host: host, OK: false,
		Error:    output.ErrorOf(errs.New(errs.SSHUnreachable, "gone", true)),
		execView: execView{ExitCode: -1}}
}

func skippedRow(host string) execManyResult {
	return execManyResult{Host: host, Skipped: true, execView: execView{ExitCode: -1}}
}

func reportOf(rows ...execManyResult) execManyReport {
	r := execManyReport{Results: rows}
	for _, row := range rows {
		switch {
		case row.Skipped:
			r.Skipped++
		case row.OK && row.ExitCode == 0:
			r.Succeeded++
		default:
			r.Failed++
		}
	}
	return r
}

func TestExecManyAggregateExitCode(t *testing.T) {
	cases := []struct {
		name string
		rows []execManyResult
		want int
	}{
		{"all green", []execManyResult{okRow("a", 0), okRow("b", 0)}, 0},
		{"remote failure is 1", []execManyResult{okRow("a", 0), okRow("b", 3)}, 1},
		{"skipped is 1", []execManyResult{okRow("a", 1), skippedRow("b")}, 1},
		{"adapter failure is 255", []execManyResult{okRow("a", 0), failingRow("b")}, 255},
		// 255 outranks 1: "a host was unreachable" must not be readable as
		// "the command ran everywhere and failed somewhere".
		{"adapter beats remote", []execManyResult{okRow("a", 7), failingRow("b")}, 255},
	}
	for _, tc := range cases {
		report := reportOf(tc.rows...)
		if got := aggregateExitCode(report); got != tc.want {
			t.Errorf("%s: aggregateExitCode = %d, want %d", tc.name, got, tc.want)
		}
	}
}

func TestExecManyCountsEveryRow(t *testing.T) {
	report := reportOf(okRow("a", 0), okRow("b", 2), failingRow("c"), skippedRow("d"))
	if report.Succeeded != 1 || report.Failed != 2 || report.Skipped != 1 {
		t.Fatalf("counts = %+v, want 1/2/1", report)
	}
	// Every target keeps its row, so the report length is the flag count; a
	// skipped target is never silently dropped from the document.
	if len(report.Results) != 4 {
		t.Fatalf("results lost a target: %d", len(report.Results))
	}
}

func TestExecManyJSONKeepsPerTargetStatus(t *testing.T) {
	report := reportOf(okRow("a", 0), skippedRow("b"))
	raw, err := json.Marshal(report)
	if err != nil {
		t.Fatal(err)
	}
	var back execManyReport
	if err := json.Unmarshal(raw, &back); err != nil {
		t.Fatal(err)
	}
	if back.Results[1].Skipped != true || back.Results[0].ExitCode != 0 {
		t.Fatalf("round trip lost row state: %s", raw)
	}
	if !strings.Contains(string(raw), `"skipped":true`) {
		t.Fatalf("skipped must be machine-visible: %s", raw)
	}
}

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

func TestSearchLineRendering(t *testing.T) {
	cases := []struct {
		row  map[string]interface{}
		want string
	}{
		{map[string]interface{}{"path": "a.go", "line": 3.0, "text": "x := 1"}, "a.go:3:x := 1"},
		{map[string]interface{}{"path": "a.go", "line": 4.0, "text": "ctx", "context": true}, "a.go-ctx"},
		{map[string]interface{}{"path": "a.go", "count": 2.0}, "a.go:2"},
		{map[string]interface{}{"path": "a.go"}, "a.go"},
	}
	for _, tc := range cases {
		if got := searchLine(tc.row); got != tc.want {
			t.Errorf("searchLine(%v) = %q, want %q", tc.row, got, tc.want)
		}
	}
}

func TestHelperUsageErrorBounds(t *testing.T) {
	for _, tc := range []struct {
		op                     string
		maxBytes, start, lines int
		limit, offset, context int
	}{
		{op: "read", maxBytes: 0},
		{op: "read", maxBytes: 10, start: 0},
		{op: "read", maxBytes: 10, lines: 0},
		{op: "grep", maxBytes: 10, limit: 0},
		{op: "grep", maxBytes: 10, offset: -1},
		{op: "grep", maxBytes: 10, limit: 1, context: -1},
	} {
		if e := helperUsageError(tc.op, tc.maxBytes, tc.start, tc.lines, tc.limit, tc.offset, tc.context); e == nil {
			t.Errorf("%+v accepted a bound the remote helper would refuse", tc)
		}
	}
	if e := helperUsageError("read", 100, 1, 200, 0, 0, 0); e != nil {
		t.Errorf("valid read rejected: %v", e)
	}
	// write and patch carry no paging bounds at all.
	if e := helperUsageError("write", 100, 0, 0, 0, 0, 0); e != nil {
		t.Errorf("valid write rejected: %v", e)
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
