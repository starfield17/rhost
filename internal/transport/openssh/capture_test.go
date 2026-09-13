package openssh

import (
	"bytes"
	"testing"
)

// Compare the retained bytes with a simple whole-stream model, including writes
// that cross the prefix boundary and tails arriving in small protocol chunks.
func TestCaptureChunks(t *testing.T) {
	for _, limit := range []int{0, 1, 64, 1024} {
		for _, chunkSize := range []int{1, 7, 255, 256, 257, 4096} {
			b := capture{limit: limit}
			var all []byte
			for i := 0; i < 20; i++ {
				chunk := bytes.Repeat([]byte{byte(i)}, chunkSize)
				n, err := b.Write(chunk)
				if err != nil || n != len(chunk) {
					t.Fatalf("Write = %d, %v", n, err)
				}
				all = append(all, chunk...)
				want := all
				if limit > 0 && len(all) > limit+tailKeep {
					want = append(append([]byte(nil), all[:limit]...), all[len(all)-tailKeep:]...)
				}
				if !bytes.Equal(b.Bytes(), want) || b.total != int64(len(all)) {
					t.Fatalf("limit=%d chunk=%d write=%d: retained bytes or total differ", limit, chunkSize, i)
				}
				if n, err := b.Write(nil); n != 0 || err != nil || !bytes.Equal(b.Bytes(), want) {
					t.Fatal("empty write changed capture")
				}
			}
		}
	}
}

func BenchmarkBoundedCapture(b *testing.B) {
	chunk := bytes.Repeat([]byte("x"), 32*1024)
	c := capture{limit: 1024}
	c.Write(chunk)
	b.SetBytes(int64(len(chunk)))
	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		c.Write(chunk)
	}
}

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
