package architecture

import (
	"bytes"
	"encoding/json"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/santhosh-tekuri/jsonschema/v5"
)

func TestRustSerializationAgainstIndependentSchema(t *testing.T) {
	path, err := filepath.Abs("../../../schemas/result-v2.schema.json")
	if err != nil {
		t.Fatal(err)
	}
	schema, err := jsonschema.Compile(path)
	if err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command("cargo", "run", "--locked", "--quiet", "--example", "contract_cases")
	cmd.Dir = "../../.."
	var stderr bytes.Buffer
	cmd.Stderr = &stderr
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("Rust DTO fixture emitter: %v: %s", err, stderr.String())
	}
	rows := bytes.Split(bytes.TrimSpace(out), []byte("\n"))
	if len(rows) != 35 {
		t.Fatalf("DTO outcome coverage changed: got %d rows, want 35", len(rows))
	}
	for i, row := range rows {
		var value interface{}
		if err := json.Unmarshal(row, &value); err != nil {
			t.Fatal(err)
		}
		if err := schema.Validate(value); err != nil {
			t.Errorf("Rust outcome %d violates v2 schema: %v\n%s", i, err, row)
		}
	}
}
