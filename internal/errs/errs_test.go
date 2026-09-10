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
