package cli

import (
	"errors"
	"fmt"
	"os"
	"text/tabwriter"

	"github.com/spf13/cobra"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// newTunnelCmd manages port forwards held by dedicated OpenSSH masters.
//
// The three subcommands are deliberately thin: every decision about what a
// forward may bind, and whether one is alive, belongs to the transport and to
// OpenSSH. What this adds is the agent-facing contract — an ID to keep, a status
// to branch on, and a JSON document for each.
func newTunnelCmd() *cobra.Command {
	group := newGroup("tunnel", "Manage persistent OpenSSH tunnels",
		"Tunnels are dedicated OpenSSH masters: they outlive the CLI process that\n"+
			"opened them, and nothing here restarts them after a reboot. The ID printed\n"+
			"by `open` is what `close` takes; `list` redisovers them on this machine.")

	var kind, listen, destination string
	var expose bool

	open := &cobra.Command{
		Use:   "open <host>",
		Short: "Open a persistent port forward",
		Long: `Open one forward on its own OpenSSH master.

kinds:
  local    --listen localhost:8080 --destination localhost:8000
           reach a service on the *remote* side through this machine
  reverse  --listen localhost:9000 --destination localhost:3000
           reach a service on *this* machine through the remote one
  socks    --listen localhost:1080
           a SOCKS5 proxy through the remote host

The bind defaults to loopback. Anything else lets other machines in through the
forward and has to be asked for with --allow-exposure. The remote sshd has its own
say: a denied reverse bind is reported as OpenSSH said it, never worked around.`,
		Args: cobra.ExactArgs(1),
		RunE: func(c *cobra.Command, args []string) error {
			audit := startAudit("tunnel.open", args[0])
			if aerr := validateTunnelOpen(kind, listen, destination, expose); aerr != nil {
				audit.fail(aerr)
				emitFailure("tunnel.open", args[0], aerr)
				return nil
			}
			t, err := tunnelClient().OpenTunnel(c.Context(), args[0], kind, listen, destination, expose)
			if err != nil {
				// Retryable, because the common causes here are transport-shaped (a
				// dropped master, a host that is briefly down). A bind sshd *refused*
				// will refuse again, so the message has to carry OpenSSH's own words —
				// which it does, verbatim (§33).
				aerr := errs.Wrap(errs.TunnelFailed, err.Error(), true, err)
				audit.fail(aerr)
				emitFailure("tunnel.open", args[0], aerr)
				return nil
			}
			audit.succeed("", kind+" "+listen+" -> "+destination, nil)
			emitTunnel("tunnel.open", args[0], t)
			return nil
		},
	}
	open.Flags().StringVar(&kind, "kind", "local", "local, reverse or socks")
	open.Flags().StringVar(&listen, "listen", "localhost:8080", "address:port to bind")
	open.Flags().StringVar(&destination, "destination", "",
		"host:port to forward to (not used for socks)")
	open.Flags().BoolVar(&expose, "allow-exposure", false,
		"permit a bind outside loopback (destructive: other machines can reach the target)")

	list := &cobra.Command{
		Use:   "list",
		Short: "List tunnels and check each master",
		Long: `List the tunnels this machine has records for, and ask each one's OpenSSH
master whether it is still up. A record whose master has gone is reported as
"stale" and left in place — it is evidence, not rhost's to discard.

An alive master is not the same as a healthy service: this checks the forward
exists, not that anything is answering on the other end.`,
		Args: cobra.NoArgs,
		RunE: func(c *cobra.Command, _ []string) error {
			rows, err := tunnelClient().ListTunnels(c.Context())
			if err != nil {
				emitFailure("tunnel.list", "", tunnelErr(err))
				return nil
			}
			emitTunnels(rows)
			return nil
		},
	}

	closeCmd := &cobra.Command{
		Use:   "close <id>",
		Short: "Close one tunnel and its own master",
		Long: `Close exactly one tunnel: its dedicated master is asked to exit and its
record is removed. The shared connection used by exec, session and job traffic is
untouched, and other tunnels keep running.`,
		Args: cobra.ExactArgs(1),
		RunE: func(c *cobra.Command, args []string) error {
			audit := startAudit("tunnel.close", "")
			err := tunnelClient().CloseTunnel(c.Context(), args[0])
			if err != nil {
				aerr := tunnelErr(err)
				audit.fail(aerr)
				emitFailure("tunnel.close", "", aerr)
				return nil
			}
			audit.succeed("", args[0], nil)
			if jsonFlag {
				writeEnvelope(output.Success("tunnel.close", "", map[string]interface{}{
					"id": args[0], "closed": true,
				}))
			} else {
				fmt.Fprintf(os.Stderr, "closed %s\n", args[0])
			}
			return nil
		},
	}

	group.AddCommand(open, list, closeCmd)
	return group
}

// tunnelClient is the OpenSSH client these commands talk through. A tunnel's
// master is chosen by the same options every other command uses, so the only
// difference between `tunnel open` and `exec` is which socket they bind.
func tunnelClient() *openssh.Client {
	return openssh.New(openssh.DefaultConfig())
}

// tunnelErr maps a transport failure onto the taxonomy. A missing record is not
// the same problem as a refused forward, and an agent that wants to clean up
// needs to be able to tell "already gone" from "could not be stopped".
func tunnelErr(err error) *errs.Error {
	if errors.Is(err, openssh.ErrInvalidTunnel) {
		return errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
	}
	if errors.Is(err, openssh.ErrNoTunnel) {
		return errs.Wrap(errs.TunnelNotFound, err.Error(), false, err)
	}
	return errs.Wrap(errs.TunnelFailed, err.Error(), true, err)
}

func validateTunnelOpen(kind, listen, destination string, expose bool) *errs.Error {
	if err := openssh.ValidateTunnel(kind, listen, destination, expose); err != nil {
		return tunnelErr(err)
	}
	return nil
}

// emitTunnel renders one tunnel; `open` returns the record it just created.
func emitTunnel(operation, host string, t openssh.Tunnel) {
	if jsonFlag {
		writeEnvelope(output.Success(operation, host, t))
		return
	}
	fmt.Printf("%s  %s  %s  %s  %s\n", t.ID, t.Status, t.Kind, t.Listen, t.Destination)
	fmt.Fprintf(os.Stderr, "keep this id: rhost tunnel close %s\n", t.ID)
}

func emitTunnels(rows []openssh.Tunnel) {
	if jsonFlag {
		writeEnvelope(output.Success("tunnel.list", "", map[string]interface{}{"tunnels": rows}))
		return
	}
	if len(rows) == 0 {
		fmt.Fprintln(os.Stderr, "no tunnels")
		return
	}
	w := tabwriter.NewWriter(os.Stdout, 0, 2, 2, ' ', 0)
	fmt.Fprintln(w, "ID\tSTATUS\tKIND\tLISTEN\tDESTINATION\tHOST")
	for _, t := range rows {
		fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\t%s\n",
			t.ID, t.Status, t.Kind, t.Listen, dash(t.Destination), t.Host)
	}
	_ = w.Flush()
}
