package cli

import (
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
