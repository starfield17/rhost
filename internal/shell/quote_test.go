package shell

import (
	"os/exec"
	"strings"
	"testing"
)

// TestQuoteRoundTrip proves Quote produces a string that bash parses back to
// exactly the original value, including the awkward cases that caused command
// injection in the reference implementation.
func TestQuoteRoundTrip(t *testing.T) {
	cases := []string{
		"",
		"simple",
		"with space",
		"it's",
		"two's and 'many' quotes '",
		"$(rm -rf /)",
		"`whoami`",
		"; rm -rf /",
		"a\nb",
		"tab\there",
		"quote'\"mixed\"`backtick`$VAR",
		"trailing backslash\\",
	}
	for _, in := range cases {
		quoted := Quote(in)
		out, err := exec.Command("/bin/bash", "-c", "printf '%s' "+quoted).Output()
		if err != nil {
			t.Fatalf("Quote(%q) = %q failed to run: %v", in, quoted, err)
		}
		if string(out) != in {
			t.Errorf("Quote(%q) round-tripped to %q (quoted=%q)", in, out, quoted)
		}
	}
}

func TestValidateEnvKey(t *testing.T) {
	good := []string{"FOO", "_x", "A1", "CUDA_VISIBLE_DEVICES"}
	for _, k := range good {
		if err := ValidateEnvKey(k); err != nil {
			t.Errorf("ValidateEnvKey(%q) = %v, want nil", k, err)
		}
	}
	bad := []string{"", "1BAD", "BAD KEY", "A=B", "a\nb", "FOO;echo"}
	for _, k := range bad {
		if err := ValidateEnvKey(k); err == nil {
			t.Errorf("ValidateEnvKey(%q) = nil, want error", k)
		}
	}
}

// TestQuoteRejectsInjection is the concrete injection regression: a value must
// never be able to introduce a second command.
func TestQuoteRejectsInjection(t *testing.T) {
	payload := "x'; touch /tmp/rhost-should-not-exist; echo '"
	cmd := "printf '%s' " + Quote(payload)
	out, err := exec.Command("/bin/bash", "-c", cmd).Output()
	if err != nil {
		t.Fatalf("run: %v", err)
	}
	if strings.TrimSpace(string(out)) != payload {
		t.Fatalf("injection payload changed: got %q want %q", out, payload)
	}
}
