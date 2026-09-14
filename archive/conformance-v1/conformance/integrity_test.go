package conformance

import (
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"testing"
)

func TestHarnessHasNoProductionImports(t *testing.T) {
	paths, err := filepath.Glob("*.go")
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range paths {
		f, err := parser.ParseFile(token.NewFileSet(), path, nil, parser.ImportsOnly)
		if err != nil {
			t.Fatal(err)
		}
		for _, i := range f.Imports {
			name, err := strconv.Unquote(i.Path.Value)
			if err != nil {
				t.Fatal(err)
			}
			if strings.HasPrefix(name, "github.com/starfield17/rhost/internal/") {
				t.Errorf("%s imports production code %s", path, name)
			}
		}
	}
}

type corpusCounts struct {
	names             []string
	assertions, skips int
}

func countCorpus(sources []string) (corpusCounts, error) {
	var count corpusCounts
	for _, source := range sources {
		f, err := parser.ParseFile(token.NewFileSet(), "corpus.go", source, 0)
		if err != nil {
			return count, err
		}
		ast.Inspect(f, func(n ast.Node) bool {
			if fn, ok := n.(*ast.FuncDecl); ok && strings.HasPrefix(fn.Name.Name, "Test") {
				count.names = append(count.names, fn.Name.Name)
			}
			if call, ok := n.(*ast.CallExpr); ok {
				if sel, ok := call.Fun.(*ast.SelectorExpr); ok {
					if ident, ok := sel.X.(*ast.Ident); ok && ident.Name == "t" {
						if strings.HasPrefix(sel.Sel.Name, "Fatal") || strings.HasPrefix(sel.Sel.Name, "Error") {
							count.assertions++
						}
						if strings.HasPrefix(sel.Sel.Name, "Skip") {
							count.skips++
						}
					}
				}
			}
			return true
		})
	}
	sort.Strings(count.names)
	return count, nil
}
func TestLiveCorpusIntegrity(t *testing.T) {
	raw, err := os.ReadFile("../fixtures/live-corpus.json")
	if err != nil {
		t.Fatal(err)
	}
	var baseline struct {
		Tests      []string `json:"tests"`
		Assertions int      `json:"minimum_assertions"`
		Skips      int      `json:"maximum_skip_sites"`
	}
	if err := json.Unmarshal(raw, &baseline); err != nil {
		t.Fatal(err)
	}
	paths, err := filepath.Glob("live*_test.go")
	if err != nil {
		t.Fatal(err)
	}
	var sources []string
	for _, path := range paths {
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		sources = append(sources, string(raw))
	}
	count, err := countCorpus(sources)
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range baseline.Tests {
		i := sort.SearchStrings(count.names, name)
		if i == len(count.names) || count.names[i] != name {
			t.Errorf("historical live test disappeared: %s", name)
		}
	}
	if count.assertions < baseline.Assertions {
		t.Errorf("live assertions decreased: %d < %d", count.assertions, baseline.Assertions)
	}
	if count.skips > baseline.Skips {
		t.Errorf("live skip sites increased: %d > %d", count.skips, baseline.Skips)
	}
	// Prove that the counter sees a deliberately disabled/renamed fixture.
	broken, err := countCorpus([]string{"package corpus; func FormerTest(t *T) { t.Skip(\"disabled\") }"})
	if err != nil {
		t.Fatal(err)
	}
	if broken.skips != 1 || len(broken.names) != 0 || broken.assertions != 0 {
		t.Fatal("integrity check missed disabled test")
	}
}

func TestHistoricalEvidenceLinks(t *testing.T) {
	raw, err := os.ReadFile("../fixtures/history.json")
	if err != nil {
		t.Fatal(err)
	}
	var rows []struct {
		ID          string  `json:"id"`
		Origin      string  `json:"origin"`
		Contract    string  `json:"contract"`
		Mechanism   string  `json:"mechanism_test"`
		Conformance *string `json:"conformance_test"`
		Gap         string  `json:"gap"`
	}
	if err := json.Unmarshal(raw, &rows); err != nil {
		t.Fatal(err)
	}
	ledger, err := os.ReadFile(filepath.Join(repositoryRoot(), "docs/CONTRACT.md"))
	if err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for _, row := range rows {
		if seen[row.ID] || row.ID == "" || row.Origin == "" {
			t.Errorf("missing or duplicate historical identity: %+v", row)
		}
		seen[row.ID] = true
		if !strings.Contains(string(ledger), "| "+row.Contract+" |") {
			t.Errorf("%s references unknown contract %s", row.ID, row.Contract)
		}
		pointers := []string{row.Mechanism}
		if row.Conformance == nil {
			if row.Gap == "" {
				t.Errorf("%s has no independent test and no explicit gap", row.ID)
			}
		} else {
			pointers = append(pointers, *row.Conformance)
		}
		for _, pointer := range pointers {
			parts := strings.Split(pointer, ":")
			if len(parts) != 2 {
				t.Fatalf("invalid test pointer %s", pointer)
			}
			f, err := parser.ParseFile(token.NewFileSet(), filepath.Join(repositoryRoot(), parts[0]), nil, 0)
			if err != nil {
				t.Fatal(err)
			}
			found := false
			for _, decl := range f.Decls {
				if fn, ok := decl.(*ast.FuncDecl); ok && fn.Name.Name == parts[1] {
					found = true
				}
			}
			if !found {
				t.Errorf("%s points at missing test %s", row.ID, pointer)
			}
		}
	}
}
