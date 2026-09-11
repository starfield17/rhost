// Package shell centralises quoting/escaping for values that reach a remote
// shell command line. There must be exactly one implementation of this logic in
// the codebase (docs/ARCHITECTURE.md §9).
package shell

import (
	"fmt"
	"regexp"
	"strings"
)

var envKeyRe = regexp.MustCompile(`^[A-Za-z_][A-Za-z0-9_]*$`)

// Quote returns a POSIX single-quoted form of s, safe to interpolate into a
// command line parsed by sh/bash/dash/ksh/zsh.
//
// The embedded-single-quote escape idiom is also understood by fish, which
// matters because sshd runs the client command through the remote account's
// *login* shell — which may be fish (see the remote login-shell note in
// README.md). We verified this against a fish login shell on a real host.
func Quote(s string) string {
	if s == "" {
		return "''"
	}
	return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'"
}

// PathQuote expands only the current remote user's home shorthand; all other
// characters remain literal. This is evaluated on the remote host, not locally.
func PathQuote(s string) string {
	if s == "~" {
		return `"$HOME"`
	}
	if strings.HasPrefix(s, "~/") {
		return `"$HOME"/` + Quote(s[2:])
	}
	return Quote(s)
}

// ValidateEnvKey rejects names that are not valid POSIX environment variable
// identifiers. This closes the `export FOO=bar\necho pwned` injection class.
func ValidateEnvKey(k string) error {
	if !envKeyRe.MatchString(k) {
		return fmt.Errorf("invalid environment variable name %q", k)
	}
	return nil
}
