// Package cli parses command-line arguments and renders output. It wires the
// application layer to the terminal; it contains no remote-control logic
// (docs/ARCHITECTURE.md §3).
package cli

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

var (
	// exitCode is the process exit status set by a command. It lets exec mirror
	// the remote command's status while still returning through cobra.
	exitCode int
	// jsonFlag is bound to the root persistent --json flag.
	jsonFlag bool
)

// Run builds and executes the command tree, returning the process exit code.
func Run() int {
	root := newRootCmd()
	root.SilenceErrors = true
	root.SilenceUsage = true

	if err := root.Execute(); err != nil {
		// Anything reaching here is a usage/flag error: application commands
		// render their own failures and return nil. The usage path still gets an
		// error.code, because agents must never have to parse English text.
		aerr := errs.New(errs.UsageError, err.Error(), false)
		if usageWantsJSON() {
			_ = output.Failure("usage", "", nil, aerr).Write(os.Stdout)
		} else {
			fmt.Fprintf(os.Stderr, "rhost: %s: %v\n", aerr.Code, err)
		}
		// 255, so that a usage error is never mistaken for a remote status.
		return 255
	}
	return exitCode
}

// usageWantsJSON reports whether --json was requested. Cobra may fail before it
// binds the flag (bad flag name, wrong arg count), so fall back to the raw argv.
func usageWantsJSON() bool {
	for _, a := range os.Args[1:] {
		if a == "--json" || a == "--json=true" {
			return true
		}
		if a == "--" { // everything after -- is the remote command, not our flags
			return false
		}
	}
	return jsonFlag
}

// newGroup builds a command group. Invoking a group with no subcommand is a usage
// error, and it used to print its help text on stdout and exit 0 — in --json mode
// that puts human prose where an agent expects exactly one JSON document
// (AGENTS.md §6). Humans still get the help; --json gets the envelope.
func newGroup(use, short, long string) *cobra.Command {
	return &cobra.Command{
		Use:   use,
		Short: short,
		Long:  long,
		RunE: func(c *cobra.Command, _ []string) error {
			if jsonFlag {
				emitFailure(use+".usage", "", errs.New(errs.UsageError,
					"rhost "+use+" needs a subcommand (see: rhost "+use+" --help)", false))
				return nil
			}
			_ = c.Help()
			return nil
		},
	}
}

func newRootCmd() *cobra.Command {
	root := &cobra.Command{
		Use:   "rhost",
		Short: "Remote host adapter for coding agents and humans",
		Long: `rhost treats an SSH-reachable machine as a reusable execution node.

It orchestrates your existing OpenSSH configuration and never duplicates SSH
authentication or host-key policy. Hosts are named exactly as you would name
them to ssh: an alias from ~/.ssh/config, a user@host, or a bare hostname.`,
	}
	root.PersistentFlags().BoolVar(&jsonFlag, "json", false, "emit machine-readable JSON on stdout")
	root.AddCommand(
		newExecCmd(),
		newDoctorCmd(),
		newHostsCmd(),
		newSessionCmd(),
		newJobCmd(),
		newFsCmd(),
		newVersionCmd(),
	)
	return root
}
