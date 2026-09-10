package output

import (
	"fmt"
	"io"
)

// Field writes a `key   value` line for human-facing output.
func Field(w io.Writer, key, value string) {
	fmt.Fprintf(w, "%-18s %s\n", key, value)
}

// Header writes a section header.
func Header(w io.Writer, title string) {
	fmt.Fprintln(w, title)
}

// OK renders a boolean capability as the human word used by `doctor`.
func OK(ok bool) string {
	if ok {
		return "OK"
	}
	return "missing"
}

// Yes renders a boolean as yes/no.
func Yes(v bool) string {
	if v {
		return "yes"
	}
	return "no"
}
