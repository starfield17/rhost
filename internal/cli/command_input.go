package cli

import (
	"fmt"
	"strings"

	"github.com/spf13/cobra"
)

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
