package output

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/starfield17/rhost/internal/errs"
)

func TestSuccessEnvelopeShape(t *testing.T) {
	var buf bytes.Buffer
	if err := Success("exec", "gpu", map[string]int{"exit_code": 3}).Write(&buf); err != nil {
		t.Fatal(err)
	}
	line := buf.String()
	if strings.Count(line, "\n") != 1 {
		t.Errorf("envelope must be exactly one line, got %q", line)
	}

	var doc map[string]interface{}
	if err := json.Unmarshal([]byte(line), &doc); err != nil {
		t.Fatalf("output is not valid JSON: %v", err)
	}
	for _, key := range []string{"schema_version", "operation", "ok", "data", "error"} {
		if _, ok := doc[key]; !ok {
			t.Errorf("missing required key %q in %s", key, line)
		}
	}
	if doc["schema_version"].(float64) != SchemaVersion {
		t.Errorf("schema_version = %v, want %d", doc["schema_version"], SchemaVersion)
	}
	if doc["ok"] != true || doc["error"] != nil {
		t.Errorf("success must carry ok=true and error=null: %s", line)
	}
	if doc["host"] != "gpu" {
		t.Errorf("host = %v, want gpu", doc["host"])
	}
}

func TestFailureEnvelopeCarriesCode(t *testing.T) {
	var buf bytes.Buffer
	aerr := errs.New(errs.SSHAuthFailed, "Permission denied (publickey).", false)
	if err := Failure("exec", "gpu", map[string]bool{"timed_out": false}, aerr).Write(&buf); err != nil {
		t.Fatal(err)
	}
	var env Envelope
	if err := json.Unmarshal(buf.Bytes(), &env); err != nil {
		t.Fatal(err)
	}
	if env.OK {
		t.Error("Failure envelope must have ok=false")
	}
	if env.Error == nil || env.Error.Code != "SSH_AUTH_FAILED" || env.Error.Retryable {
		t.Errorf("error payload = %+v", env.Error)
	}
}

// TestHostOmittedWhenEmpty pins the `host,omitempty` behaviour agents rely on.
func TestHostOmittedWhenEmpty(t *testing.T) {
	var buf bytes.Buffer
	if err := Success("hosts", "", nil).Write(&buf); err != nil {
		t.Fatal(err)
	}
	if strings.Contains(buf.String(), `"host"`) {
		t.Errorf("host should be omitted when empty, got %s", buf.String())
	}
}

// TestTaxonomyMatchesSchema is the drift gate: every code the adapter can emit
// must be declared in the published envelope schema, and the schema must not
// advertise codes that do not exist. Agents branch on error.code, so a code
// missing from the schema is an unhandleable response.
func TestTaxonomyMatchesSchema(t *testing.T) {
	path := filepath.Join("..", "..", "schemas", "result-v1.schema.json")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read schema: %v", err)
	}
	var schema struct {
		Properties struct {
			Error struct {
				OneOf []struct {
					Properties struct {
						Code struct {
							Enum []string `json:"enum"`
						} `json:"code"`
					} `json:"properties"`
				} `json:"oneOf"`
			} `json:"error"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(raw, &schema); err != nil {
		t.Fatalf("parse schema: %v", err)
	}
	declared := schema.Properties.Error.OneOf[1].Properties.Code.Enum

	want := []string{
		string(errs.UsageError),
		string(errs.ConfigInvalid),
		string(errs.HostUnknown),
		string(errs.SSHUnreachable),
		string(errs.SSHAuthFailed),
		string(errs.HostKeyFailed),
		string(errs.RemoteDependencyMissing),
		string(errs.RemoteCommandTimeout),
		string(errs.SessionNotFound),
		string(errs.SessionUnhealthy),
		string(errs.JobNotFound),
		string(errs.JobStateUnknown),
		string(errs.TransferFailed),
		string(errs.SyncRejected),
		string(errs.UnsupportedRemoteOS),
		string(errs.Internal),
	}
	sort.Strings(want)
	got := append([]string{}, declared...)
	sort.Strings(got)

	if strings.Join(got, ",") != strings.Join(want, ",") {
		t.Errorf("schema error.code enum is out of sync with internal/errs:\n schema: %v\n  errs: %v", got, want)
	}
}

// TestSchemaRequiredKeysExistOnTheGoType keeps the Go struct and the schema's
// `required` list pointing at the same fields.
func TestSchemaRequiredKeysExistOnTheGoType(t *testing.T) {
	var buf bytes.Buffer
	if err := Success("version", "", nil).Write(&buf); err != nil {
		t.Fatal(err)
	}
	var doc map[string]interface{}
	if err := json.Unmarshal(buf.Bytes(), &doc); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"schema_version", "operation", "ok", "data", "error"} {
		if _, ok := doc[key]; !ok {
			t.Errorf("required key %q absent", key)
		}
	}
}
