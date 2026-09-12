package app

import (
	"bytes"
	"errors"
	"testing"
	"time"

	"github.com/starfield17/rhost/internal/errs"
)

func TestExecutionTimeoutRetryDependsOnConfirmedCleanup(t *testing.T) {
	for _, confirmed := range []bool{false, true} {
		e := executionTimeout(2*time.Second, confirmed)
		if e.Code != errs.RemoteCommandTimeout {
			t.Fatalf("cleanup=%v code=%s", confirmed, e.Code)
		}
		if e.Retryable != confirmed {
			t.Fatalf("cleanup=%v retryable=%v", confirmed, e.Retryable)
		}
	}
}

func TestProtocolStreamForwardsIncrementallyAndHidesMarkers(t *testing.T) {
	const nonce = "0123456789abcdef0123456789abcdef"
	var dst bytes.Buffer
	w := newProtocolStream(nonce, &dst, 0)
	parts := []string{
		"profile noise\x00__RHOST_BEG",
		"IN_" + nonce + "__\nfirst line\n",
		"second line",
		"\x00__RHOST_DONE_" + nonce + "__:7\n",
	}
	for i, part := range parts {
		if _, err := w.Write([]byte(part)); err != nil {
			t.Fatal(err)
		}
		if i == 1 && dst.String() != "first line\n" {
			t.Fatalf("output was not forwarded before completion: %q", dst.String())
		}
	}
	code, ok := w.finish()
	if !ok || code != 7 {
		t.Fatalf("completion = %d, %v", code, ok)
	}
	got, total, truncated := w.result()
	if got != "first line\nsecond line" || dst.String() != got || total != int64(len(got)) || truncated {
		t.Fatalf("result = %q, %q, %d, %v", got, dst.String(), total, truncated)
	}
}

func TestProtocolStreamCaptureIsBounded(t *testing.T) {
	const nonce = "0123456789abcdef0123456789abcdef"
	w := newProtocolStream(nonce, nil, 4)
	_, _ = w.Write([]byte("\x00__RHOST_BEGIN_" + nonce + "__\nabcdef\x00__RHOST_DONE_" + nonce + "__:0\n"))
	code, ok := w.finish()
	got, total, truncated := w.result()
	if !ok || code != 0 || got != "abcd" || total != 6 || !truncated {
		t.Fatalf("result = code %d ok %v body %q total %d truncated %v", code, ok, got, total, truncated)
	}
}

type failingWriter struct{}

func (failingWriter) Write([]byte) (int, error) { return 0, errors.New("closed") }

func TestProtocolStreamRecordsOutputWriteFailure(t *testing.T) {
	const nonce = "0123456789abcdef0123456789abcdef"
	w := newProtocolStream(nonce, failingWriter{}, -1)
	cancelled := false
	w.cancel = func() { cancelled = true }
	if _, err := w.Write([]byte("\x00__RHOST_BEGIN_" + nonce + "__\noutput")); err == nil {
		t.Fatal("write failure was hidden")
	}
	if w.err() == nil {
		t.Fatal("write failure was not retained")
	}
	if !cancelled {
		t.Fatal("write failure did not cancel the SSH process")
	}
}
