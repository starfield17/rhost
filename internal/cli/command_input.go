package cli

import (
	"fmt"

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

func (v *shellCommandValue) validate(fixedArgs int) cobra.PositionalArgs {
	return func(cmd *cobra.Command, args []string) error {
		if cmd.ArgsLenAtDash() >= 0 {
			return fmt.Errorf("%s does not accept a command after --; use --command <string>", cmd.CommandPath())
		}
		if err := cobra.ExactArgs(fixedArgs)(cmd, args); err != nil {
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
