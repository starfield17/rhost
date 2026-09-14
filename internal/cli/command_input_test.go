package cli

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/audit"
)

func TestShellCommandInputGrammar(t *testing.T) {
	tests := []struct {
		name  string
		build func() *cobra.Command
		args  []string
		ok    bool
	}{
		{name: "exec long", build: newExecCmd, args: []string{"example-host", "--command", "printf '%s' '--json'"}, ok: true},
		{name: "exec short and interspersed", build: newExecCmd, args: []string{"example-host", "-c", "pwd", "--cwd", "/var/log"}, ok: true},
		{name: "exec command file", build: newExecCmd, args: []string{"example-host", "--command-file", "inspect.sh"}, ok: true},
		{name: "conflicting sources", build: newExecCmd, args: []string{"example-host", "--command", "true", "--command-file", "inspect.sh"}},
		{name: "stdin command file", build: newExecCmd, args: []string{"example-host", "--command-file", "-"}},
		{name: "stream requires json", build: newExecCmd, args: []string{"example-host", "--command", "true", "--stream"}},
		{name: "session flags after command", build: newSessionExecCmd, args: []string{"example-host", "dev", "--command", "x=42", "--timeout", "1s"}, ok: true},
		{name: "missing command", build: newExecCmd, args: []string{"example-host"}},
		{name: "empty command", build: newExecCmd, args: []string{"example-host", "--command", ""}},
		{name: "duplicate command", build: newExecCmd, args: []string{"example-host", "--command", "true", "-c", "false"}},
		{name: "extra operand", build: newSessionExecCmd, args: []string{"example-host", "dev", "extra", "--command", "true"}},
		{name: "legacy delimiter", build: newSessionExecCmd, args: []string{"example-host", "dev", "--", "sleep 1"}},
		{name: "empty delimiter", build: newExecCmd, args: []string{"example-host", "--command", "true", "--"}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cmd := tt.build()
			err := cmd.ParseFlags(tt.args)
			if err == nil {
				err = cmd.ValidateArgs(cmd.Flags().Args())
			}
			if tt.ok && err != nil {
				t.Fatalf("valid input rejected: %v", err)
			}
			if !tt.ok && err == nil {
				t.Fatal("invalid input accepted")
			}
		})
	}
}

func TestJSONStreamKeepsEnvelopeOnStdoutAndProgramInputOnStdin(t *testing.T) {
	t.Cleanup(saveGlobals())
	dir := t.TempDir()
	ssh := filepath.Join(dir, "ssh")
	if err := os.WriteFile(ssh, []byte(`#!/bin/sh
for remote do :; done
remote=$(printf '%s' "$remote" | sed 's/^exec setsid /exec /')
eval "$remote"
`), 0o700); err != nil {
		t.Fatal(err)
	}
	program := filepath.Join(dir, "inspect script.sh")
	if err := os.WriteFile(program, []byte("IFS= read -r line\nprintf 'out:%s' \"$line\"\nprintf 'err:%s' \"$line\" >&2\n"), 0o600); err != nil {
		t.Fatal(err)
	}
	input, err := os.CreateTemp(dir, "stdin")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := input.WriteString("value\n"); err != nil {
		t.Fatal(err)
	}
	if _, err := input.Seek(0, 0); err != nil {
		t.Fatal(err)
	}
	defer input.Close()
	diagnostics, err := os.CreateTemp(dir, "stderr")
	if err != nil {
		t.Fatal(err)
	}
	defer diagnostics.Close()
	oldIn, oldErr := os.Stdin, os.Stderr
	os.Stdin, os.Stderr = input, diagnostics
	defer func() { os.Stdin, os.Stderr = oldIn, oldErr }()
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	t.Setenv("RHOST_STATE_DIR", dir)
	t.Setenv("RHOST_AUDIT", "0")
	os.Args = []string{"rhost", "--json", "exec", "example-host", "--command-file", program, "--stream"}
	code := 0
	stdout := captureStdout(t, func() { code = Run() })
	if _, err := diagnostics.Seek(0, 0); err != nil {
		t.Fatal(err)
	}
	live, err := os.ReadFile(diagnostics.Name())
	if err != nil {
		t.Fatal(err)
	}
	if code != 0 {
		t.Fatalf("exit = %d stdout=%q stderr=%q", code, stdout, live)
	}
	var env struct {
		OK   bool                            `json:"ok"`
		Data struct{ Stdout, Stderr string } `json:"data"`
	}
	if err := json.Unmarshal([]byte(stdout), &env); err != nil {
		t.Fatalf("stdout is not one JSON document: %v\n%s", err, stdout)
	}
	if !env.OK || env.Data.Stdout != "out:value" || env.Data.Stderr != "err:value" {
		t.Fatalf("envelope = %+v", env)
	}
	if !bytes.Contains(live, []byte("out:value")) || !bytes.Contains(live, []byte("err:value")) {
		t.Fatalf("live stderr = %q", live)
	}
}

func TestReadCommandFileValidation(t *testing.T) {
	dir := t.TempDir()
	write := func(name string, data []byte) string {
		path := filepath.Join(dir, name)
		if err := os.WriteFile(path, data, 0o600); err != nil {
			t.Fatal(err)
		}
		return path
	}
	valid := write("with spaces.sh", []byte("printf '%s\\n' \"$HOME\"\ncat <<'EOF'\nhello\nEOF\n"))
	if got, err := readCommandFile(valid); err != nil || !bytes.Contains([]byte(got), []byte("<<'EOF'")) {
		t.Fatalf("valid file = %q err=%v", got, err)
	}
	for _, tc := range []struct {
		name string
		data []byte
	}{
		{"empty", nil},
		{"nul", []byte("true\x00false")},
		{"large", bytes.Repeat([]byte("x"), 64*1024+1)},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if _, err := readCommandFile(write(tc.name, tc.data)); err == nil {
				t.Fatal("invalid file accepted")
			}
		})
	}
	if _, err := readCommandFile(dir); err == nil {
		t.Fatal("directory accepted")
	}
}

func TestLegacyCommandIsRejectedBeforeAuditOrSSH(t *testing.T) {
	t.Cleanup(saveGlobals())

	dir := t.TempDir()
	marker := filepath.Join(dir, "ssh-called")
	ssh := filepath.Join(dir, "ssh")
	if err := os.WriteFile(ssh, []byte("#!/bin/sh\n: > \"$RHOST_TEST_SSH_MARKER\"\nexit 255\n"), 0o700); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	t.Setenv("RHOST_TEST_SSH_MARKER", marker)
	t.Setenv("RHOST_STATE_DIR", dir)
	t.Setenv("RHOST_AUDIT", "1")

	os.Args = []string{"rhost", "exec", "example-host", "--", "sleep 1; echo ok", "--json"}
	if code := Run(); code != 255 {
		t.Fatalf("exit = %d, want usage status 255", code)
	}
	if _, err := os.Stat(marker); !os.IsNotExist(err) {
		t.Fatalf("invalid command reached ssh: %v", err)
	}
	if _, err := os.Stat(audit.Path(dir)); !os.IsNotExist(err) {
		t.Fatalf("invalid command wrote audit state: %v", err)
	}
}
