package app

import (
	"encoding/json"
	"os/exec"
	"strings"
	"testing"
)

func TestHelperOutputBudgetCoversJSONEscaping(t *testing.T) {
	// Use Python's actual ensure_ascii encoder, not Go's different escaping rules.
	cmd := exec.Command("python3", "-c", `import json; print(json.dumps({'content': ('\x01' * 1023 + '\n') * 200}, ensure_ascii=True))`)
	data, err := cmd.Output()
	if err != nil {
		t.Fatal(err)
	}
	budget := helperOutputBudget(map[string]interface{}{"max_bytes": 256 * 1024})
	if len(data) > budget {
		t.Fatalf("response %d exceeds budget %d", len(data), budget)
	}
	var result struct{ Content string }
	if err := json.Unmarshal(data, &result); err != nil {
		t.Fatal(err)
	}
	if result.Content != strings.Repeat(strings.Repeat("\x01", 1023)+"\n", 200) {
		t.Fatal("content changed")
	}
	if max := helperOutputBudget(map[string]interface{}{"max_bytes": 8 * 1024 * 1024}); max > maxExecOutputBytes {
		t.Fatalf("budget %d exceeds exec limit", max)
	}
}
