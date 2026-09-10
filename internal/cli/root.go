// Package cli parses command-line arguments and renders output. It wires the
// application layer to the terminal; it contains no remote-control logic
// (docs/ARCHITECTURE.md §3).
package cli

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
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
		// render their own failures and return nil.
		fmt.Fprintf(os.Stderr, "rhost: %v\n", err)
		return 2
	}
	return exitCode
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
		newVersionCmd(),
	)
	return root
}
