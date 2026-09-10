package shell

import "unicode/utf8"

// IncompleteUTF8Suffix returns the number of trailing bytes of b that form an
// incomplete UTF-8 sequence (a lead byte without all of its continuation bytes
// yet), or 0 when b ends on a rune boundary.
//
// rhost reads session logs in fixed-size chunks, so a read can stop in the
// middle of a multi-byte rune. Callers use this to hold those bytes back until
// the next read delivers the rest, instead of emitting U+FFFD into JSON.
func IncompleteUTF8Suffix(b []byte) int {
	// Walk back at most UTFMax bytes looking for the lead byte of the final rune.
	for i := 1; i <= utf8.UTFMax && i <= len(b); i++ {
		c := b[len(b)-i]
		if c < utf8.RuneSelf {
			return 0 // last byte is ASCII: the buffer ends on a boundary
		}
		if c&0xC0 == 0xC0 { // lead byte
			want := utf8RuneLen(c)
			if want > i {
				return i // incomplete: only i of want bytes are present
			}
			return 0 // complete, or an invalid lead byte we leave alone
		}
		// A continuation byte: keep walking back toward the lead byte.
	}
	return 0
}

// utf8RuneLen reports the encoded length implied by a UTF-8 lead byte.
func utf8RuneLen(c byte) int {
	switch {
	case c&0xE0 == 0xC0:
		return 2
	case c&0xF0 == 0xE0:
		return 3
	case c&0xF8 == 0xF0:
		return 4
	default:
		return 1
	}
}
