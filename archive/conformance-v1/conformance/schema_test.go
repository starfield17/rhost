package conformance

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func v2Fixtures(t *testing.T) []struct {
	Name   string                 `json:"name"`
	Result map[string]interface{} `json:"result"`
} {
	t.Helper()
	raw, err := os.ReadFile("../fixtures/result-v2-valid.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixtures []struct {
		Name   string                 `json:"name"`
		Result map[string]interface{} `json:"result"`
	}
	if err := json.Unmarshal(raw, &fixtures); err != nil {
		t.Fatal(err)
	}
	return fixtures
}
func TestSchemaV2Fixtures(t *testing.T) {
	seen := map[string]bool{}
	for _, fixture := range v2Fixtures(t) {
		t.Run(fixture.Name, func(t *testing.T) {
			raw, err := json.Marshal(fixture.Result)
			if err != nil {
				t.Fatal(err)
			}
			if err := validateEnvelope(raw); err != nil {
				t.Fatal(err)
			}
			seen[fixture.Name] = true
			// Every envelope field required by the public contract is independently tested.
			for _, key := range []string{"schema_version", "operation", "ok", "data", "error"} {
				var invalid map[string]interface{}
				if err := json.Unmarshal(raw, &invalid); err != nil {
					t.Fatal(err)
				}
				delete(invalid, key)
				bad, err := json.Marshal(invalid)
				if err != nil {
					t.Fatal(err)
				}
				if validateEnvelope(bad) == nil {
					t.Errorf("missing %s accepted", key)
				}
			}
		})
	}
	raw, err := os.ReadFile(filepath.Join(repositoryRoot(), "schemas/result-v2.schema.json"))
	if err != nil {
		t.Fatal(err)
	}
	var schema struct {
		Properties struct {
			Operation struct {
				Enum []string `json:"enum"`
			} `json:"operation"`
		} `json:"properties"`
	}
	if err := json.Unmarshal(raw, &schema); err != nil {
		t.Fatal(err)
	}
	for _, op := range schema.Properties.Operation.Enum {
		if !seen[op] {
			t.Errorf("operation %s has no fixture", op)
		}
	}
}

func TestSchemaV2RejectsFalseEvidence(t *testing.T) {
	tests := map[string]func(map[string]interface{}){
		"unknown-success": func(v map[string]interface{}) {
			v["data"].(map[string]interface{})["execution"] = map[string]interface{}{"status": "unknown"}
		},
		"unknown-with-exit": func(v map[string]interface{}) {
			v["data"].(map[string]interface{})["execution"] = map[string]interface{}{"status": "unknown", "exit_code": 0}
		},
		"negative-exit": func(v map[string]interface{}) {
			v["data"].(map[string]interface{})["execution"] = map[string]interface{}{"status": "completed", "exit_code": -1}
		},
		"missing-exit": func(v map[string]interface{}) {
			v["data"].(map[string]interface{})["execution"] = map[string]interface{}{"status": "completed"}
		},
		"null-success": func(v map[string]interface{}) { v["data"] = nil },
		"success-with-error": func(v map[string]interface{}) {
			v["error"] = map[string]interface{}{"code": "INTERNAL", "message": "bad", "retryable": false}
		},
		"job": func(v map[string]interface{}) { v["operation"] = "job.start" },
		"job-error": func(v map[string]interface{}) {
			v["ok"] = false
			v["error"] = map[string]interface{}{"code": "JOB_NOT_FOUND", "message": "bad", "retryable": false}
		},
		"retryable-timeout": func(v map[string]interface{}) {
			v["ok"] = false
			v["error"] = map[string]interface{}{"code": "REMOTE_COMMAND_TIMEOUT", "message": "timeout", "retryable": true}
		},
		"cleanup-before-submission": func(v map[string]interface{}) {
			v["ok"] = false
			v["error"] = map[string]interface{}{"code": "SSH_UNREACHABLE", "message": "failed", "retryable": true}
			d := v["data"].(map[string]interface{})
			d["execution"] = map[string]interface{}{"status": "not_started"}
			d["cleanup"] = map[string]interface{}{"status": "confirmed_stopped"}
		},
		"unknown-error-with-completion": func(v map[string]interface{}) {
			v["ok"] = false
			v["error"] = map[string]interface{}{"code": "REMOTE_EXECUTION_UNKNOWN", "message": "unknown", "retryable": false}
		},
	}
	for name, change := range tests {
		t.Run(name, func(t *testing.T) {
			v := v2Fixtures(t)[0].Result
			change(v)
			raw, err := json.Marshal(v)
			if err != nil {
				t.Fatal(err)
			}
			if validateEnvelope(raw) == nil {
				t.Fatalf("invalid evidence accepted: %s", raw)
			}
		})
	}
	for _, raw := range []string{`{}`, `{"schema_version":false}`, `{"schema_version":3}`, `null`, `[]`, `{} {}`} {
		if validateEnvelope([]byte(raw)) == nil {
			t.Errorf("invalid document accepted: %s", raw)
		}
	}
}

func TestSchemaV2KeepsKnownExitAfterInterruption(t *testing.T) {
	for _, code := range []string{"REMOTE_COMMAND_TIMEOUT", "REMOTE_COMMAND_CANCELLED", "OUTPUT_WRITE_FAILED"} {
		v := v2Fixtures(t)[0].Result
		v["ok"] = false
		v["error"] = map[string]interface{}{"code": code, "message": "interrupted", "retryable": false}
		raw, err := json.Marshal(v)
		if err != nil {
			t.Fatal(err)
		}
		var e envelope
		if err := json.Unmarshal(raw, &e); err != nil {
			t.Fatal(err)
		}
		if e.num(t, "exit_code") != 0 {
			t.Fatal("known foreground completion lost")
		}
	}
}

func TestSchemaDecoderRejectsDuplicateKeys(t *testing.T) {
	for _, raw := range []string{`{"schema_version":1,"schema_version":2}`, `{"data":{"execution":{"status":"unknown","status":"completed"}}}`} {
		if rejectDuplicateKeys([]byte(raw)) == nil {
			t.Fatalf("ambiguous document accepted: %s", raw)
		}
	}
}
