package openssh

// capture is the bounded sink behind ssh's stdout and stderr.
//
// An unbounded buffer here turns `rhost exec host --command 'yes | head -c 999999999'`
// into an unbounded allocation in the CLI, and the JSON would carry all of it. But
// simply keeping the first N bytes would break the completion protocol: the
// marker that carries the command's real exit status is the *last* thing on
// stdout, so a capped reader that drops the end would report a successful command
// as a transport failure.
//
// So this keeps a prefix and a fixed protocol-sized suffix while draining every
// byte, and counts the total. The tail only has to be long enough to hold one
// completion marker, which is a property of rhost's own protocol, not of the
// command — that is what makes the bound safe rather than merely small.
type capture struct {
	// limit is the number of leading bytes to keep; 0 keeps everything.
	limit int
	head  []byte
	tail  []byte
	// total counts every byte written, kept or not.
	total int64
}

// tailKeep is the size of the retained suffix: one completion marker plus room
// for the newline and status that follow it.
const tailKeep = 256

func (b *capture) Write(p []byte) (int, error) {
	n := len(p)
	b.total += int64(n)
	if b.limit <= 0 {
		b.head = append(b.head, p...)
		return n, nil
	}
	room := b.limit - len(b.head)
	if room > len(p) {
		room = len(p)
	}
	if room > 0 {
		b.head = append(b.head, p[:room]...)
		p = p[room:]
	}
	if len(p) > 0 {
		if b.tail == nil {
			b.tail = make([]byte, 0, tailKeep)
		}
		if len(p) >= tailKeep {
			// The new chunk replaces the entire suffix; never allocate for
			// bytes that will immediately be discarded.
			b.tail = append(b.tail[:0], p[len(p)-tailKeep:]...)
		} else {
			if overflow := len(b.tail) + len(p) - tailKeep; overflow > 0 {
				b.tail = b.tail[:copy(b.tail, b.tail[overflow:])]
			}
			b.tail = append(b.tail, p...)
		}
	}
	return n, nil
}

// Bytes returns what was kept: the prefix, followed by the retained suffix.
//
// Those two are contiguous whenever the total stayed within limit+tailKeep, which
// is the interesting case — an output that only just exceeded the cap arrives
// whole. Past that, the middle is genuinely gone, and the caller's own truncation
// to `limit` is what stops a reader from seeing the join.
func (b *capture) Bytes() []byte {
	if len(b.tail) == 0 {
		return b.head
	}
	out := make([]byte, 0, len(b.head)+len(b.tail))
	out = append(out, b.head...)
	return append(out, b.tail...)
}
