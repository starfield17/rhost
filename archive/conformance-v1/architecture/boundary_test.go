package architecture

import (
	"os"
	"path/filepath"
	"regexp"
	"testing"
)

// A conservative lexical guard complements compiling domain as a standalone
// library: dependency roots and I/O modules cannot be imported via grouped use.
var rustComments = regexp.MustCompile(`(?s)/\*.*?\*/|//[^\n]*`)
var rustStrings = regexp.MustCompile(`"(?:\\.|[^"\\])*"`)
var domainForbidden = regexp.MustCompile(`\b(?:serde|serde_json|tokio|unsafe|fs|io|process|net|env|thread|include|include_str|include_bytes)\b|\bcrate\s*::|\bsuper\s*::\s*super\b`)

func domainViolation(source string) bool {
	return domainForbidden.MatchString(rustStrings.ReplaceAllString(rustComments.ReplaceAllString(source, ""), ""))
}
func TestRustDomainBoundary(t *testing.T) {
	paths, err := filepath.Glob("../../../src/domain/*.rs")
	if err != nil {
		t.Fatal(err)
	}
	if len(paths) == 0 {
		t.Fatal("domain source missing")
	}
	for _, path := range paths {
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		if domainViolation(string(raw)) {
			t.Errorf("domain crosses its pure boundary: %s", path)
		}
	}
	for _, bad := range []string{"use crate::output;", "use std::{io, fmt};", "use std::process::Command;", "use serde::Serialize;", "use super::super::output;"} {
		if !domainViolation(bad) {
			t.Errorf("boundary guard accepted %s", bad)
		}
	}
	if domainViolation("// use serde::Serialize;\nuse std::fmt;\nlet value = \"process\";") {
		t.Fatal("boundary guard treated text as dependency")
	}
}
