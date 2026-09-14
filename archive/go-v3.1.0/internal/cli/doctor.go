package cli

import (
	"fmt"
	"os"
	"time"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/output"
)

func newDoctorCmd() *cobra.Command {
	var timeout time.Duration
	var fresh bool
	cmd := &cobra.Command{
		Use:   "doctor <host>",
		Short: "Probe a remote host's capabilities",
		Long: `Probe a host and report what it supports before relying on it.

The probe is read-only and runs through the same exec path used by real work, so
a successful doctor run also proves batch authentication, bash, setsid, and the
completion-marker protocol work on this host.`,
		Args: exactNamedArgs("<host>"),
		RunE: func(cmd *cobra.Command, args []string) error {
			host := args[0]
			audit := startAudit("doctor", host)
			a := app.NewDefault()
			res, aerr := a.Doctor(cmd.Context(), app.DoctorOptions{Host: host, Timeout: timeout, Fresh: fresh})
			if aerr != nil {
				audit.fail(aerr)
				if jsonFlag {
					writeEnvelope(output.Failure("doctor", host, res, aerr))
					exitCode = adapterExitCode(aerr)
				} else {
					emitFailure("doctor", host, aerr)
					fmt.Fprintf(os.Stderr, "master_status=%s control_path=%s\n",
						res.Connection.MasterStatus, res.Connection.ControlPath)
				}
				return nil
			}
			audit.succeed("", "", nil)
			renderDoctor(res)
			return nil
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 60*time.Second, "probe timeout")
	cmd.Flags().BoolVar(&fresh, "fresh", false, "probe through an independent SSH connection")
	return cmd
}

func renderDoctor(r app.DoctorResult) {
	if jsonFlag {
		writeEnvelope(output.Success("doctor", r.Host, r))
		return
	}
	w := os.Stdout
	fmt.Fprintln(w, r.Host)
	output.Field(w, "SSH", "OK (batch auth)")
	output.Field(w, "Shared master", string(r.Connection.MasterStatus))
	if r.ConnectionReused != nil {
		output.Field(w, "Connection reused", output.Yes(*r.ConnectionReused))
	}
	output.Field(w, "OS", r.OS)
	output.Field(w, "Kernel", r.Kernel)
	output.Field(w, "Arch", r.Arch)
	output.Field(w, "User", r.User)
	output.Field(w, "Login shell", r.LoginShell)
	output.Field(w, "State dir", r.StateDir+"  writable="+output.Yes(r.StateDirWritable))
	output.Field(w, "WSL", output.Yes(r.WSL))
	fmt.Fprintln(w)
	for _, c := range []string{"bash", "tmux", "setsid", "ps", "rsync", "sha256sum", "base64", "stty", "flock", "python3", "realpath"} {
		output.Field(w, c, output.OK(r.Capabilities[c]))
	}
}

func newConnectionCmd() *cobra.Command {
	cmd := newGroup("connection", "Inspect and reset shared SSH connections", `Connection controls apply only to rhost's ordinary shared OpenSSH master.
They do not affect dedicated tunnel masters.`)
	cmd.AddCommand(newConnectionStatusCmd(), newConnectionResetCmd())
	return cmd
}

func newConnectionStatusCmd() *cobra.Command {
	return &cobra.Command{
		Use: "status <host>", Short: "Inspect the shared SSH master", Args: exactNamedArgs("<host>"),
		RunE: func(cmd *cobra.Command, args []string) error {
			res := app.NewDefault().ConnectionStatus(cmd.Context(), args[0])
			if jsonFlag {
				writeEnvelope(output.Success("connection.status", args[0], res))
			} else {
				output.Field(os.Stdout, "Master status", string(res.MasterStatus))
				output.Field(os.Stdout, "Control path", res.ControlPath)
				if res.MasterPID != 0 {
					output.Field(os.Stdout, "Master PID", fmt.Sprint(res.MasterPID))
				}
				if res.Diagnostic != "" {
					output.Field(os.Stdout, "Diagnostic", res.Diagnostic)
				}
			}
			return nil
		},
	}
}

func newConnectionResetCmd() *cobra.Command {
	return &cobra.Command{
		Use: "reset <host>", Short: "Stop the shared SSH master accepting new requests", Args: exactNamedArgs("<host>"),
		RunE: func(cmd *cobra.Command, args []string) error {
			audit := startAudit("connection.reset", args[0])
			res, aerr := app.NewDefault().ConnectionReset(cmd.Context(), args[0])
			if aerr != nil {
				audit.fail(aerr)
				if jsonFlag {
					writeEnvelope(output.Failure("connection.reset", args[0], res, aerr))
					exitCode = adapterExitCode(aerr)
				} else {
					emitFailure("connection.reset", args[0], aerr)
				}
				return nil
			}
			audit.succeed("", "stop shared OpenSSH master", nil)
			if jsonFlag {
				writeEnvelope(output.Success("connection.reset", args[0], res))
			} else if res.Stopped {
				fmt.Fprintln(os.Stdout, "shared SSH master stopped accepting new requests")
			} else {
				fmt.Fprintln(os.Stdout, "no shared SSH master")
			}
			return nil
		},
	}
}
