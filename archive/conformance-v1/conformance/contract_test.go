package conformance

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/santhosh-tekuri/jsonschema/v5"
)

var schemaOnce sync.Once
var schemas map[int]*jsonschema.Schema
var schemaError error

func validateEnvelope(raw []byte) error {
	if err := rejectDuplicateKeys(raw); err != nil {
		return err
	}
	schemaOnce.Do(func() {
		schemas = make(map[int]*jsonschema.Schema)
		for _, v := range []int{1, 2} {
			c := jsonschema.NewCompiler()
			path, err := filepath.Abs(filepath.Join(repositoryRoot(), fmt.Sprintf("schemas/result-v%d.schema.json", v)))
			if v == 1 {
				path = filepath.Join(repositoryRoot(), "archive/go-v3.1.0/schemas/result-v1.schema.json")
			}
			if err != nil {
				schemaError = err
				return
			}
			schemas[v], err = c.Compile(path)
			if err != nil {
				schemaError = err
				return
			}
		}
	})
	if schemaError != nil {
		return schemaError
	}
	var doc map[string]interface{}
	d := json.NewDecoder(bytes.NewReader(raw))
	d.UseNumber()
	if err := d.Decode(&doc); err != nil {
		return err
	}
	if err := d.Decode(new(interface{})); err != io.EOF {
		return fmt.Errorf("expected exactly one JSON document")
	}
	version, present := doc["schema_version"].(json.Number)
	if !present {
		return fmt.Errorf("missing schema version")
	}
	v, err := version.Int64()
	if err != nil {
		return fmt.Errorf("missing or invalid schema version")
	}
	schema := schemas[int(v)]
	if schema == nil {
		return fmt.Errorf("unsupported schema version %d", v)
	}
	if err := schema.Validate(doc); err != nil {
		return err
	}
	ok, present := doc["ok"].(bool)
	if !present || (ok && doc["error"] != nil) || (!ok && doc["error"] == nil) {
		return fmt.Errorf("inconsistent adapter outcome")
	}
	if ok && doc["data"] == nil {
		return fmt.Errorf("successful result has no data")
	}
	if ok && v == 1 && (doc["operation"] == "exec" || doc["operation"] == "session.exec") {
		data, valid := doc["data"].(map[string]interface{})
		if !valid {
			return fmt.Errorf("completed execution has no result object")
		}
		status, valid := data["exit_code"].(json.Number)
		if !valid {
			return fmt.Errorf("completed execution has no known exit")
		}
		code, err := status.Int64()
		if err != nil || code < 0 || code > 255 {
			return fmt.Errorf("completed execution has invalid exit")
		}
	}
	return nil
}

// UnmarshalJSON validates the wire version before any behavior-level access.
// It never fabricates completion evidence or repairs a candidate's JSON.
func (e *envelope) UnmarshalJSON(raw []byte) error {
	if err := validateEnvelope(raw); err != nil {
		return err
	}
	type wire envelope
	var value wire
	if err := json.Unmarshal(raw, &value); err != nil {
		return err
	}
	*e = envelope(value)
	return nil
}

// observableField maps only documented v2 representation changes. Callers
// retain the original v1 behavior assertions; absent evidence remains absent.
func (e envelope) observableField(key string) (json.RawMessage, error) {
	var data map[string]json.RawMessage
	if err := json.Unmarshal(e.Data, &data); err != nil {
		return nil, err
	}
	path := []string{key}
	if e.SchemaVersion == 2 && (e.Operation == "exec" || e.Operation == "session.exec") {
		switch key {
		case "exit_code":
			path = []string{"execution", "exit_code"}
		case "stdout", "stderr":
			if e.Operation == "session.exec" {
				path = []string{"output", "content"}
			} else {
				path = []string{"output", key, "content"}
			}
		case "stdout_bytes", "stderr_bytes", "stdout_truncated", "stderr_truncated":
			parts := strings.Split(key, "_")
			path = []string{"output", parts[0], parts[1]}
		case "timed_out", "cancelled":
			code := "REMOTE_COMMAND_TIMEOUT"
			if key == "cancelled" {
				code = "REMOTE_COMMAND_CANCELLED"
			}
			return json.Marshal(e.Error != nil && e.Error.Code == code)
		case "cleanup_confirmed":
			var cleanup struct {
				Status string `json:"status"`
			}
			if err := json.Unmarshal(data["cleanup"], &cleanup); err != nil {
				return nil, err
			}
			return json.Marshal(cleanup.Status == "confirmed_stopped")
		}
	}
	for i, part := range path {
		raw, ok := data[part]
		if !ok {
			return nil, fmt.Errorf("missing data.%s", strings.Join(path, "."))
		}
		if i == len(path)-1 {
			return raw, nil
		}
		if err := json.Unmarshal(raw, &data); err != nil {
			return nil, err
		}
	}
	return nil, fmt.Errorf("empty field path")
}

func (e envelope) assertUnknownSessionExit(t *testing.T) {
	t.Helper()
	if e.SchemaVersion == 1 {
		raw, err := e.observableField("exit_code")
		if err != nil || string(raw) != "null" {
			t.Fatalf("unknown session exit must be null in v1: %s (%v)", e.Data, err)
		}
		return
	}
	var data struct {
		Execution struct {
			Status string `json:"status"`
			Exit   *int   `json:"exit_code"`
		} `json:"execution"`
	}
	if err := json.Unmarshal(e.Data, &data); err != nil {
		t.Fatal(err)
	}
	if data.Execution.Status != "not_started" || data.Execution.Exit != nil {
		t.Fatalf("busy session must not have executed: %s", e.Data)
	}
}

func resolveBinary(path string) (string, error) {
	if path == "" {
		return "", fmt.Errorf("set RHOST_BIN=/path/to/rhost")
	}
	if !filepath.IsAbs(path) && os.Getenv("RHOST_BIN_ROOT") != "" {
		path = filepath.Join(os.Getenv("RHOST_BIN_ROOT"), path)
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	info, err := os.Stat(abs)
	if err != nil {
		return "", fmt.Errorf("RHOST_BIN: %w", err)
	}
	if !info.Mode().IsRegular() || info.Mode().Perm()&0111 == 0 {
		return "", fmt.Errorf("RHOST_BIN must be an executable regular file")
	}
	return abs, nil
}

func quoteShell(value string) string { return "'" + strings.ReplaceAll(value, "'", "'\"'\"'") + "'" }

func TestHarnessBinarySelection(t *testing.T) {
	if _, err := resolveBinary(""); err == nil {
		t.Fatal("missing binary accepted")
	}
	if _, err := resolveBinary(t.TempDir()); err == nil {
		t.Fatal("directory accepted")
	}
	dir := t.TempDir()
	path := filepath.Join(dir, "candidate with spaces")
	if err := os.WriteFile(path, []byte("#!/bin/sh\nexit 0\n"), 0600); err != nil {
		t.Fatal(err)
	}
	if _, err := resolveBinary(path); err == nil {
		t.Fatal("non-executable accepted")
	}
	if err := os.Chmod(path, 0700); err != nil {
		t.Fatal(err)
	}
	got, err := resolveBinary(path)
	if err != nil || got != path {
		t.Fatalf("selected %q: %v", got, err)
	}
	t.Setenv("RHOST_BIN_ROOT", dir)
	if got, err := resolveBinary("candidate with spaces"); err != nil || got != path {
		t.Fatalf("relative spaced path=%q: %v", got, err)
	}
	if _, err := resolveBinary(filepath.Join(dir, "absent")); err == nil {
		t.Fatal("missing path accepted")
	}
}

func TestConformanceCLI(t *testing.T) {
	toolDir := t.TempDir()
	toolLog := filepath.Join(t.TempDir(), "unexpected-tools")
	for _, name := range []string{"ssh", "scp", "rsync"} {
		if err := os.WriteFile(filepath.Join(toolDir, name), []byte("#!/bin/sh\nprintf called >> \"$RHOST_TOOL_CALL_LOG\"\nexit 99\n"), 0700); err != nil {
			t.Fatal(err)
		}
	}
	t.Cleanup(func() {
		if raw, err := os.ReadFile(toolLog); err == nil {
			t.Errorf("CLI validation invoked a remote tool: %s", raw)
		} else if !os.IsNotExist(err) {
			t.Error(err)
		}
	})
	c := liveCLI{bin: buildBinary(t)}.withEnv("PATH="+toolDir+":"+os.Getenv("PATH"), "RHOST_TOOL_CALL_LOG="+toolLog, "HOME="+t.TempDir(), "RHOST_STATE_DIR="+t.TempDir(), "RHOST_CACHE_DIR="+t.TempDir())
	t.Run("version", func(t *testing.T) {
		e := c.mustJSON(t, "version", "--json")
		if e.Operation != "version" || e.str(t, "version") == "" {
			t.Fatal("missing binary identity")
		}
		if e.num(t, "schema_version") != e.SchemaVersion {
			t.Fatal("version disagrees with envelope")
		}
	})
	t.Run("hosts", func(t *testing.T) {
		e := c.mustJSON(t, "hosts", "--json")
		var hosts []json.RawMessage
		e.field(t, "hosts", &hosts)
		if len(hosts) != 0 {
			t.Fatal("empty home unexpectedly discovered hosts")
		}
	})
	t.Run("usage", func(t *testing.T) {
		for _, args := range [][]string{{"--json"}, {"exec", "--json"}, {"job", "--json"}, {"--json", "--not-a-flag"}} {
			c.wantErrorCode(t, "USAGE_ERROR", args...)
		}
	})
	t.Run("group-usage", func(t *testing.T) {
		for _, group := range []string{"session", "connection", "fs", "tunnel"} {
			e := c.wantErrorCode(t, "USAGE_ERROR", group, "--json")
			if e.Operation != group+".usage" {
				t.Fatalf("group usage operation=%s", e.Operation)
			}
		}
	})
	t.Run("command-validation", func(t *testing.T) {
		c.wantErrorCode(t, "USAGE_ERROR", "exec", "gpu", "--json", "--command", "")
		for _, command := range []string{"   "} {
			c.wantErrorCode(t, "CONFIG_INVALID", "exec", "gpu", "--json", "--command", command)
		}
	})
}

func TestHarnessCleanupUsesSelectedBinary(t *testing.T) {
	oldBinary, oldBinDir := selectedBinary, liveBinDir
	liveHarness.Lock()
	oldDirs := liveHarness.remoteDirs
	liveHarness.remoteDirs = []string{"/tmp/rhost-conformance-owned"}
	liveHarness.Unlock()
	defer func() {
		selectedBinary, liveBinDir = oldBinary, oldBinDir
		liveHarness.Lock()
		liveHarness.remoteDirs = oldDirs
		liveHarness.Unlock()
	}()
	dir := t.TempDir()
	log := filepath.Join(dir, "cleanup-args")
	candidate := filepath.Join(dir, "external candidate")
	script := "#!/bin/sh\nprintf '%s\\n' \"$@\" > " + quoteShell(log) + "\n"
	if err := os.WriteFile(candidate, []byte(script), 0700); err != nil {
		t.Fatal(err)
	}
	selectedBinary = candidate
	liveBinDir = ""
	if err := cleanupLiveResources(); err != nil {
		t.Fatal(err)
	}
	args, err := os.ReadFile(log)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Contains(args, []byte("rhost-conformance-owned")) {
		t.Fatalf("cleanup did not use selected candidate: %s", args)
	}
	if _, err := os.Stat(candidate); err != nil {
		t.Fatal("external candidate was deleted")
	}
	if err := os.WriteFile(candidate, []byte("#!/bin/sh\nexit 9\n"), 0700); err != nil {
		t.Fatal(err)
	}
	if cleanupLiveResources() == nil {
		t.Fatal("cleanup failure was swallowed")
	}
}

// Reject ambiguous JSON before schema validation loses duplicate object keys.
func rejectDuplicateKeys(raw []byte) error {
	d := json.NewDecoder(bytes.NewReader(raw))
	var value func() error
	value = func() error {
		token, err := d.Token()
		if err != nil {
			return err
		}
		delimiter, composite := token.(json.Delim)
		if !composite {
			return nil
		}
		switch delimiter {
		case '{':
			keys := map[string]bool{}
			for d.More() {
				token, err := d.Token()
				if err != nil {
					return err
				}
				key, ok := token.(string)
				if !ok || keys[key] {
					return fmt.Errorf("duplicate or invalid JSON object key")
				}
				keys[key] = true
				if err := value(); err != nil {
					return err
				}
			}
		case '[':
			for d.More() {
				if err := value(); err != nil {
					return err
				}
			}
		default:
			return fmt.Errorf("unexpected JSON delimiter")
		}
		_, err = d.Token()
		return err
	}
	if err := value(); err != nil {
		return err
	}
	if _, err := d.Token(); err != io.EOF {
		return fmt.Errorf("expected exactly one JSON document")
	}
	return nil
}

// The harness is frozen here; the working Rust checkout is outside the archive.
func repositoryRoot() string {
	if root := os.Getenv("RHOST_REPO_ROOT"); root != "" {
		return root
	}
	root, err := filepath.Abs("../../..")
	if err != nil {
		panic(err)
	}
	return root
}
