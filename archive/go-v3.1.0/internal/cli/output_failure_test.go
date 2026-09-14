package cli

import (
	"os"
	"strings"
	"testing"
)

func TestJSONDeliveryFailureExits255(t *testing.T) {
	t.Cleanup(saveGlobals())
	oldFailure := outputFailed
	defer func() { outputFailed = oldFailure }()
	sink, err := os.Open(os.DevNull) // read-only descriptor: writes fail
	if err != nil {
		t.Fatal(err)
	}
	defer sink.Close()
	oldOut, oldErr := os.Stdout, os.Stderr
	diagnostics, err := os.CreateTemp(t.TempDir(), "stderr")
	if err != nil {
		t.Fatal(err)
	}
	defer diagnostics.Close()
	os.Stdout, os.Stderr = sink, diagnostics
	defer func() { os.Stdout, os.Stderr = oldOut, oldErr }()
	os.Args = []string{"rhost", "version", "--json"}
	if code := Run(); code != 255 {
		t.Fatalf("exit = %d", code)
	}
	data, err := os.ReadFile(diagnostics.Name())
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(data), "OUTPUT_WRITE_FAILED") {
		t.Fatalf("diagnostic = %q", data)
	}
	// A later invocation resets the delivery failure independently of remote status.
	os.Stdout = diagnostics
	if code := Run(); code != 0 {
		t.Fatalf("next exit = %d", code)
	}
}
