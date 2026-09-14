package cli

import (
	"bytes"
	"fmt"
	"os"
	"strings"

	"github.com/spf13/cobra"
)

const maxCommandFileBytes = 64 * 1024

// shellCommandValue keeps a shell program as one exact CLI value. It rejects
// repeated assignment instead of silently letting the last --command win.
type shellCommandValue struct {
	value string
	set   bool
}

func (v *shellCommandValue) Set(value string) error {
	if v.set {
		return fmt.Errorf("--command may be provided exactly once")
	}
	v.value = value
	v.set = true
	return nil
}

func (v *shellCommandValue) String() string { return v.value }
func (v *shellCommandValue) Type() string   { return "shell-command" }

func bindShellCommand(cmd *cobra.Command, value *shellCommandValue) {
	cmd.Flags().VarP(value, "command", "c", "exact shell program to execute")
}

type commandFileValue struct {
	value string
	set   bool
}

func (v *commandFileValue) Set(value string) error {
	if v.set {
		return fmt.Errorf("--command-file may be provided exactly once")
	}
	v.value, v.set = value, true
	return nil
}
func (v *commandFileValue) String() string { return v.value }
func (v *commandFileValue) Type() string   { return "path" }

func bindCommandFile(cmd *cobra.Command, value *commandFileValue) {
	cmd.Flags().Var(value, "command-file", "local file containing the exact shell program")
}

func readCommandFile(path string) (string, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return "", fmt.Errorf("read --command-file: %w", err)
	}
	if !info.Mode().IsRegular() {
		return "", fmt.Errorf("--command-file must be a regular file")
	}
	if info.Size() > maxCommandFileBytes {
		return "", fmt.Errorf("--command-file exceeds 65536 bytes")
	}
	b, err := os.ReadFile(path)
	if err != nil {
		return "", fmt.Errorf("read --command-file: %w", err)
	}
	if len(b) == 0 {
		return "", fmt.Errorf("--command-file must not be empty")
	}
	if len(b) > maxCommandFileBytes {
		return "", fmt.Errorf("--command-file exceeds 65536 bytes")
	}
	if bytes.IndexByte(b, 0) >= 0 {
		return "", fmt.Errorf("--command-file contains NUL")
	}
	return string(b), nil
}

func validateExecInput(command *shellCommandValue, file *commandFileValue, stream *bool) cobra.PositionalArgs {
	return func(cmd *cobra.Command, args []string) error {
		if cmd.ArgsLenAtDash() >= 0 {
			return fmt.Errorf("%s does not accept a command after --; use --command or --command-file", cmd.CommandPath())
		}
		if err := exactNamedArgs("<host>")(cmd, args); err != nil {
			return err
		}
		if command.set == file.set {
			return fmt.Errorf("%s requires exactly one of --command or --command-file", cmd.CommandPath())
		}
		if command.set && command.value == "" {
			return fmt.Errorf("--command must not be empty")
		}
		if file.set && (file.value == "" || file.value == "-") {
			return fmt.Errorf("--command-file requires a local file path, not stdin")
		}
		if *stream && !jsonFlag {
			return fmt.Errorf("--stream requires --json")
		}
		return nil
	}
}

// exactNamedArgs keeps Cobra's exact-arity rejection while naming the operands
// a caller forgot. The names are the public CLI grammar, not parser internals.
func exactNamedArgs(names ...string) cobra.PositionalArgs {
	return func(cmd *cobra.Command, args []string) error {
		if len(args) < len(names) {
			return fmt.Errorf("%s requires %s", cmd.CommandPath(), strings.Join(names, " "))
		}
		return cobra.ExactArgs(len(names))(cmd, args)
	}
}

func (v *shellCommandValue) validate(fixedArgs ...string) cobra.PositionalArgs {
	return func(cmd *cobra.Command, args []string) error {
		if cmd.ArgsLenAtDash() >= 0 {
			return fmt.Errorf("%s does not accept a command after --; use --command <string>", cmd.CommandPath())
		}
		if err := exactNamedArgs(fixedArgs...)(cmd, args); err != nil {
			return err
		}
		if !v.set {
			return fmt.Errorf("%s requires --command <string>", cmd.CommandPath())
		}
		if v.value == "" {
			return fmt.Errorf("--command must not be empty")
		}
		return nil
	}
}
