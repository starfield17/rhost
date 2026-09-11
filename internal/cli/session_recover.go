package cli

import (
	"fmt"
	"os"
	"time"

	"github.com/spf13/cobra"
	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

// newSessionRecoverCmd brings a wedged managed shell back to a prompt.
//
// It is the one command in the session family that is allowed to interrupt what
// is running: it sends C-c, then proves the shell answers again by running a
// command that cannot fail. It never recreates the session — the cwd, exported
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

Read data.session_preserved. A false means the shell did not answer: the pane is
still owned by whatever was running, the result carries SESSION_BUSY rather than
waiting out a timeout, and the next step is ` + "`session read`" + ` — to see what
it is doing — or ` + "`session close`" + `, not another blind command.`,
		Args: cobra.ExactArgs(2),
		RunE: func(c *cobra.Command, args []string) error {
			host, session := args[0], args[1]
			a := app.NewDefault()

			audit := startAudit("session.recover", host)
			if aerr := a.SessionSend(c.Context(), host, session, "", "C-c", timeout); aerr != nil {
				audit.fail(aerr)
				emitFailure("session.recover", host, aerr)
				return nil
			}
			audit.succeed("", "key C-c", nil)

			// Ctrl-C reaches the program first and the prompt follows, so the pane
			// can still look busy for a moment after a successful interrupt. A busy
			// refusal is retried briefly; anything else is reported as it is.
			var res app.SessionExecResult
			var aerr *errs.Error
			for attempt := 0; attempt < 4; attempt++ {
				res, aerr = a.SessionExec(c.Context(), host, session, ":", timeout)
				if aerr == nil || aerr.Code != errs.SessionBusy {
					break
				}
				time.Sleep(250 * time.Millisecond)
			}
			preserved := aerr == nil && res.SessionPreserved
			if aerr == nil && !preserved {
				// The probe command ran and never came back with a shell prompt:
				// that is the session being unhealthy, not a transport failure.
				aerr = errs.New(errs.SessionUnhealthy,
					"the session did not answer after Ctrl-C; read it or close it", true)
			}
			data := map[string]interface{}{
				"session_id":        session,
				"session_preserved": preserved,
			}
			if aerr != nil {
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
