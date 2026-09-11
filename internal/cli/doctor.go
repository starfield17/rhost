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
	cmd := &cobra.Command{
		Use:   "doctor <host>",
		Short: "Probe a remote host's capabilities",
		Long: `Probe a host and report what it supports before relying on it.

The probe is read-only and runs through the same exec path used by real work, so
a successful doctor run also proves batch authentication, bash, setsid, and the
completion-marker protocol work on this host.`,
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			host := args[0]
			audit := startAudit("doctor", host)
			a := app.NewDefault()
			res, aerr := a.Doctor(cmd.Context(), host, timeout)
			if aerr != nil {
				audit.fail(aerr)
				emitFailure("doctor", host, aerr)
				return nil
			}
			audit.succeed("", "", nil)
			renderDoctor(res)
			return nil
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 60*time.Second, "probe timeout")
	return cmd
}

func renderDoctor(r app.DoctorResult) {
	if jsonFlag {
		_ = output.Success("doctor", r.Host, r).Write(os.Stdout)
		return
	}
	w := os.Stdout
	fmt.Fprintln(w, r.Host)
	output.Field(w, "SSH", "OK (batch auth)")
	output.Field(w, "OS", r.OS)
	output.Field(w, "Kernel", r.Kernel)
	output.Field(w, "Arch", r.Arch)
	output.Field(w, "User", r.User)
	output.Field(w, "Login shell", r.LoginShell)
	output.Field(w, "State dir", r.StateDir+"  writable="+output.Yes(r.StateDirWritable))
	output.Field(w, "WSL", output.Yes(r.WSL))
	output.Field(w, "Process identity", output.Yes(r.Capabilities["pid_identity"]))
	output.Field(w, "systemd --user", output.Yes(r.SystemdUser))
	fmt.Fprintln(w)
	for _, c := range []string{"bash", "tmux", "nohup", "setsid", "ps", "rsync", "sha256sum", "base64", "stty", "systemctl", "git", "nvidia-smi", "flock"} {
		output.Field(w, c, output.OK(r.Capabilities[c]))
	}
}
