package cli

import (
	"fmt"
	"os"
	"time"

	"github.com/spf13/cobra"
	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/output"
)

// newSessionRecoverCmd brings a wedged managed shell back to a prompt.
//
// It is the one command in the session family that is allowed to interrupt what
// is running: it sends C-c, then requires the managed shell in the foreground
// and a fresh prompt marker. It never recreates the session — the cwd, exported
// variables and shell history are the thing being recovered — and if the probe
// does not come back, `session_preserved: false` says so instead of pretending the
// shell is usable.
//
// This is also what `session exec` does on its own after a timeout, which is why
// the two report the same field.
func newSessionRecoverCmd() *cobra.Command {
	var timeout time.Duration
	cmd := &cobra.Command{
		Use:   "recover <host> <session>",
		Short: "Interrupt what is running and verify the shell answers",
		Long: `Send Ctrl-C to a managed session and check that its shell is responsive again.

Anything waiting at that prompt is interrupted: this is how you stop a program that
is holding a session, not a read-only health check. The session itself is never
recreated, so its working directory, environment and history survive — or the
result says they did not.

Read data.session_preserved. A false means shell readiness was not proven.
If another program still owns the pane after the recovery budget, the result
carries SESSION_BUSY and data.foreground. Recover never exits a REPL for you.
The next step is ` + "`session read`" + ` — to see what
it is doing — or ` + "`session close`" + `, not another blind command.`,
		Args: cobra.ExactArgs(2),
		RunE: func(c *cobra.Command, args []string) error {
			host, session := args[0], args[1]
			a := app.NewDefault()

			audit := startAudit("session.recover", host)
			res, aerr := a.SessionRecover(c.Context(), host, session, timeout)
			data := map[string]interface{}{
				"session_id":        session,
				"session_preserved": res.SessionPreserved,
			}
			if res.Foreground != "" {
				data["foreground"] = res.Foreground
			}
			if aerr != nil {
				audit.fail(aerr)
				// Not emitFailure: the partial data is the answer here, and an agent
				// needs session_preserved even when the recovery failed.
				if jsonFlag {
					_ = output.Failure("session.recover", host, data, aerr).Write(os.Stdout)
				} else {
					fmt.Fprintf(os.Stderr, "rhost: %s: %s\n", aerr.Code, aerr.Message)
				}
				exitCode = adapterExitCode(aerr)
				return nil
			}
			audit.succeed("", "key C-c and responsiveness probe", nil)
			if jsonFlag {
				_ = output.Success("session.recover", host, data).Write(os.Stdout)
			} else {
				fmt.Println("session is responsive")
			}
			return nil
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second,
		"deadline for the interrupt and for the probe command")
	return cmd
}
