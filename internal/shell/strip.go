package shell

import (
	"regexp"
	"strings"
)

// Only the sequence families rhost actually produces need stripping: CSI
// (colours, cursor moves, bracketed-paste toggles) and OSC (the OSC 133
// command-boundary markers).
var (
	ansiCSI = regexp.MustCompile("\x1b\\[[0-9;?]*[A-Za-z]")
	ansiOSC = regexp.MustCompile("\x1b\\][^\x07]*\x07")
)

// StripANSI removes terminal control sequences and normalises line endings so
// captured session output is readable. OSC is stripped before CSI so the BEL
// terminator is not left behind.
func StripANSI(s string) string {
	s = ansiOSC.ReplaceAllString(s, "")
	s = ansiCSI.ReplaceAllString(s, "")
	s = strings.ReplaceAll(s, "\r\n", "\n")
	s = strings.ReplaceAll(s, "\r", "")
	return s
}
