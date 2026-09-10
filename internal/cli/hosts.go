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

Wildcard and negated patterns are omitted. This does not contact any host.`,
		Args: cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			a := app.NewDefault()
			infos, aerr := a.Hosts()
			if aerr != nil {
				emitFailure("hosts", "", aerr, 1)
				return nil
			}
			if jsonFlag {
				_ = output.Success("hosts", "", map[string]interface{}{"hosts": infos}).Write(os.Stdout)
				return nil
			}
			if len(infos) == 0 {
				fmt.Fprintln(os.Stderr, "no host aliases found in ~/.ssh/config")
				return nil
			}
			for _, h := range infos {
				fmt.Fprintln(os.Stdout, h.Alias)
			}
			return nil
		},
	}
}
