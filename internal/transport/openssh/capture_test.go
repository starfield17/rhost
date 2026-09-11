package openssh

import (
	"bytes"
	"testing"
)

func TestBoundedCaptureRetainsCompletion(t *testing.T) {
	b := capture{limit: 1024}
	b.Write([]byte("\n" + beginToken("test") + "\n"))
	for i := 0; i < 1024; i++ {
		b.Write(bytes.Repeat([]byte("x"), 1024))
	}
	b.Write([]byte("\n" + markerToken("test") + ":7\n"))
	_, code, ok := ParseMarker(b.Bytes(), "test")
	if !ok || code != 7 || len(b.Bytes()) > 1280 || b.total < 1024*1024 {
		t.Fatalf("capture lost bounds or completion: %d %v %d", code, ok, len(b.Bytes()))
	}
}
