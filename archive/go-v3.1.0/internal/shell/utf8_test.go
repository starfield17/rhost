package shell

import "testing"

func TestIncompleteUTF8Suffix(t *testing.T) {
	cases := []struct {
		name string
		in   []byte
		want int
	}{
		{"empty", nil, 0},
		{"ascii", []byte("abc"), 0},
		{"complete two byte", []byte("é"), 0},
		{"truncated two byte", []byte{0xC3}, 1},
		{"truncated three byte, one continuation", []byte{0xE2, 0x82}, 2},
		{"complete three byte", []byte{0xE2, 0x82, 0xAC}, 0},
		{"truncated four byte", []byte{0xF0, 0x9F, 0x98}, 3},
		{"complete four byte", []byte{0xF0, 0x9F, 0x98, 0x80}, 0},
		{"ascii then truncated lead", []byte("hi\xC3"), 1},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := IncompleteUTF8Suffix(tc.in); got != tc.want {
				t.Errorf("IncompleteUTF8Suffix(%v) = %d, want %d", tc.in, got, tc.want)
			}
		})
	}
}
