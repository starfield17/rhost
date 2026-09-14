package errs

import (
	"errors"
	"testing"
)

func TestFromCoercion(t *testing.T) {
	adapterErr := New(ConfigInvalid, "bad key", false)
	if got := From(adapterErr); got != adapterErr {
		t.Error("From must pass an *Error through unchanged")
	}

	plain := errors.New("boom")
	got := From(plain)
	if got.Code != Internal || got.Message != "boom" || got.Retryable {
		t.Errorf("From(plain) = %+v, want INTERNAL/boom/false", got)
	}
	if !errors.Is(got.Unwrap(), plain) {
		t.Error("From must retain the cause for diagnostics")
	}
	if From(nil) != nil {
		t.Error("From(nil) must be nil so callers can write From(maybeErr)")
	}
}

func TestIsMatchesCodeOnlyForThatCode(t *testing.T) {
	err := New(SSHUnreachable, "down", true)
	if !Is(err, SSHUnreachable) {
		t.Error("Is should match its own code")
	}
	if Is(err, SSHAuthFailed) {
		t.Error("Is must not match a different code")
	}
	if Is(errors.New("x"), SSHUnreachable) {
		t.Error("Is must not match a plain error")
	}
	if Is(nil, SSHUnreachable) {
		t.Error("Is(nil) must be false")
	}
}

func TestErrorStringForms(t *testing.T) {
	err := New(HostKeyFailed, "host key verification failed", false)
	if err.Error() != "host key verification failed" {
		t.Errorf("Error() = %q, want the message (code is separate)", err.Error())
	}
	if err.String() != "HOST_KEY_FAILED: host key verification failed" {
		t.Errorf("String() = %q", err.String())
	}
}

// The code list is a contract an agents branches on, so its two failure modes are
// written down: a code declared twice (which would make `error.code` ambiguous and
// still pass the schema comparison if the schema had been edited the same way), and
// a code whose name does not look like the others.
func TestCodesAreUniqueAndWellFormed(t *testing.T) {
	seen := map[Code]bool{}
	for _, c := range allCodes {
		if seen[c] {
			t.Errorf("code %s is declared twice", c)
		}
		seen[c] = true
		if !KnownCode(string(c)) {
			t.Errorf("KnownCode(%s) is false for a declared code", c)
		}
		for _, r := range string(c) {
			if !(r == '_' || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9')) {
				t.Errorf("code %q must be UPPER_SNAKE so it reads the same in every language", c)
				break
			}
		}
	}
	if len(Codes()) != len(allCodes) {
		t.Errorf("Codes() dropped one: %d vs %d", len(Codes()), len(allCodes))
	}
	if !sortsStrings(Codes()) {
		t.Error("Codes() must be sorted so a contract diff is stable")
	}
	if KnownCode("NOT_A_CODE") || KnownCode("") || KnownCode("ssh_unreachable") {
		t.Error("KnownCode must answer for the published set only")
	}
}

func sortsStrings(xs []string) bool {
	for i := 1; i < len(xs); i++ {
		if xs[i-1] > xs[i] {
			return false
		}
	}
	return true
}
