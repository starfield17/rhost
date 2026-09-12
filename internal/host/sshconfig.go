// Package host discovers usable SSH targets. OpenSSH configuration remains the
// source of truth (docs/PROJECT_OVERVIEW.md); rhost only reads it to list
// aliases for humans and agents.
//
// The listing is best-effort by design: it follows Include directives and skips
// wildcard/negated patterns, but it is not an implementation of ssh_config's
// full grammar (Match, quoted includes and the system-wide file are not
// modelled), and it never claims a host is reachable. `ssh <target>` is the
// authority; this exists so an agent can see what the user probably meant.
package host

import (
	"bufio"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Info describes one configured host alias.
type Info struct {
	Alias  string `json:"alias"`
	Source string `json:"source,omitempty"`
}

type Discovery struct {
	Hosts       []Info   `json:"hosts"`
	ConfigFound bool     `json:"config_found"`
	Complete    bool     `json:"complete"`
	Warnings    []string `json:"warnings"`
}

// Aliases returns concrete Host aliases declared in the user's OpenSSH client
// config, following Include directives. Wildcard/negated patterns are skipped
// because they do not name a single host.
func Aliases() (Discovery, error) {
	home, _ := os.UserHomeDir()
	base := filepath.Join(home, ".ssh", "config")

	seen := map[string]bool{}
	out := []Info{} // never nil: `hosts --json` must render [], not null
	visited := map[string]bool{}
	result := Discovery{Hosts: out, Complete: true, Warnings: []string{}}
	if _, err := os.Stat(base); err == nil {
		result.ConfigFound = true
	} else if !os.IsNotExist(err) {
		return result, err
	}

	var walk func(path string)
	walk = func(path string) {
		if visited[path] {
			return
		}
		visited[path] = true
		f, err := os.Open(path)
		if err != nil {
			result.Complete = false
			result.Warnings = append(result.Warnings, "could not read an included SSH config file")
			return
		}
		defer f.Close()

		sc := bufio.NewScanner(f)
		for sc.Scan() {
			line := strings.TrimSpace(sc.Text())
			if line == "" || strings.HasPrefix(line, "#") {
				continue
			}
			key, rest := splitKeyword(line)
			switch strings.ToLower(key) {
			case "host":
				for _, tok := range strings.Fields(rest) {
					if tok == "" || strings.ContainsAny(tok, "*?!") {
						continue
					}
					if !seen[tok] {
						seen[tok] = true
						out = append(out, Info{Alias: tok, Source: path})
					}
				}
			case "include":
				for _, pat := range strings.Fields(rest) {
					for _, m := range glob(pat, home) {
						walk(m)
					}
				}
			}
		}
		if err := sc.Err(); err != nil {
			result.Complete = false
			result.Warnings = append(result.Warnings, "could not completely read an SSH config file")
		}
	}
	if result.ConfigFound {
		walk(base)
	}

	sort.Slice(out, func(i, j int) bool { return out[i].Alias < out[j].Alias })
	result.Hosts = out
	return result, nil
}

// splitKeyword splits an ssh_config line into its keyword and the remainder,
// accepting both "Key value" and "Key=value" forms.
func splitKeyword(line string) (string, string) {
	line = strings.TrimSpace(line)
	i := strings.IndexAny(line, " \t=")
	if i < 0 {
		return line, ""
	}
	return line[:i], strings.TrimSpace(strings.TrimLeft(line[i:], " \t="))
}

// glob expands an Include path. Absolute paths are used as-is; everything else
// is relative to ~/.ssh, matching OpenSSH's Include semantics.
func glob(pattern, home string) []string {
	p := pattern
	if strings.HasPrefix(p, "~/") {
		p = filepath.Join(home, p[2:])
	} else if !filepath.IsAbs(p) {
		p = filepath.Join(home, ".ssh", p)
	}
	matches, err := filepath.Glob(p)
	if err != nil {
		return nil
	}
	return matches
}
