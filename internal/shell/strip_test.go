package shell

import "testing"

func TestStripANSI(t *testing.T) {
	cases := []struct{ in, want string }{
		{"\x1b]133;C\x07hello\r\n", "hello\n"},
		{"\x1b]133;D;0\x07", ""},
		{"\x1b[?2004h$ ls\x1b[?2004l\r\n", "$ ls\n"},
		{"plain text\n", "plain text\n"},
		{"a\rb\n", "ab\n"},
	}
	for _, c := range cases {
		if got := StripANSI(c.in); got != c.want {
			t.Errorf("StripANSI(%q) = %q, want %q", c.in, got, c.want)
		}
	}
}
