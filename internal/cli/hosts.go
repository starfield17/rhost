package cli

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/output"
)

func newHostsCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "hosts",
		Short: "List SSH config host aliases",
		Long: `List concrete Host aliases from your OpenSSH client config.

This is best-effort discovery, not OpenSSH's own resolution: it reads the user
config and its Include files, and does not implement Match, quoted includes, or
the system-wide config. A host you can reach may be missing from the list, and a
listed alias is not a promise that the connection succeeds: plain ssh is the
authority. Wildcard and negated patterns are omitted. No host is contacted.`,
		Args: cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			a := app.NewDefault()
			result, aerr := a.Hosts()
			if aerr != nil {
				emitFailure("hosts", "", aerr)
				return nil
			}
			if jsonFlag {
				_ = output.Success("hosts", "", result).Write(os.Stdout)
				return nil
			}
			if len(result.Hosts) == 0 {
				fmt.Fprintln(os.Stderr, "no host aliases found in ~/.ssh/config")
				return nil
			}
			for _, h := range result.Hosts {
				fmt.Fprintln(os.Stdout, h.Alias)
			}
			return nil
		},
	}
}
