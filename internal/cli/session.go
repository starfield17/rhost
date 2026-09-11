package cli

import (
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

func newSessionCmd() *cobra.Command {
	cmd := newGroup("session", "Manage persistent tmux-backed sessions", `A session is a persistent interactive shell on the remote host, owned by
remote tmux, so it survives the CLI process and SSH disconnects.

Use a session only when cwd, environment, or interactive terminal state must
persist across calls. For ordinary commands, prefer `+"`rhost exec`"+`.

`+"`session exec`"+` runs a command in the managed shell, and refuses with
SESSION_BUSY when a program — a REPL, a debugger, an editor — owns the pane,
because pasting a command into that program is not what the caller asked for.
`+"`session send`"+` is the raw terminal path for exactly that case, and
`+"`session recover`"+` is how a stuck pane is interrupted.`)
	cmd.AddCommand(
		newSessionCreateCmd(),
		newSessionListCmd(),
		newSessionExecCmd(),
		newSessionSendCmd(),
		newSessionReadCmd(),
		newSessionCloseCmd(),
		newSessionAttachCmd(),
		newSessionRecoverCmd(),
	)
	return cmd
}

func newSessionCreateCmd() *cobra.Command {
	var (
		name    string
		cwd     string
		shell   string
		timeout time.Duration
	)
	cmd := &cobra.Command{
		Use:   "create <host>",
		Short: "Create a persistent session",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			host := args[0]
			audit := startAudit("session.create", host)
			a := app.NewDefault()
			info, aerr := a.SessionCreate(cmd.Context(), host, name, cwd, shell, timeout)
			if aerr != nil {
				audit.fail(aerr)
				emitFailure("session.create", host, aerr)
				return nil
			}
			audit.succeed(cwd, "", nil)
			if jsonFlag {
				_ = output.Success("session.create", host, info).Write(os.Stdout)
			} else {
				fmt.Printf("created session %s (name %s)\n", info.ID, info.Name)
			}
			return nil
		},
	}
	cmd.Flags().StringVar(&name, "name", "", "human-friendly session name")
	cmd.Flags().StringVar(&cwd, "cwd", "", "initial working directory")
	cmd.Flags().StringVar(&shell, "shell", "bash", "shell to run in the session (bash)")
	cmd.Flags().DurationVar(&timeout, "timeout", 60*time.Second, "creation timeout")
	return cmd
}

func newSessionListCmd() *cobra.Command {
	var timeout time.Duration
	cmd := &cobra.Command{
		Use:   "list <host>",
		Short: "List sessions and their liveness",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			host := args[0]
			a := app.NewDefault()
			sessions, aerr := a.SessionList(cmd.Context(), host, timeout)
			if aerr != nil {
				emitFailure("session.list", host, aerr)
				return nil
			}
			if jsonFlag {
				_ = output.Success("session.list", host, map[string]interface{}{"sessions": sessions}).Write(os.Stdout)
				return nil
			}
			if len(sessions) == 0 {
				fmt.Fprintln(os.Stderr, "no sessions")
				return nil
			}
			fmt.Printf("%-16s %-16s %-7s %s\n", "ID", "NAME", "STATUS", "CWD")
			for _, s := range sessions {
				fmt.Printf("%-16s %-16s %-7s %s\n", s.ID, s.Name, s.Status, s.InitialCwd)
			}
			return nil
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "list timeout")
	return cmd
}

func newSessionExecCmd() *cobra.Command {
	var timeout time.Duration
	cmd := &cobra.Command{
		Use:   "exec <host> <session> [--] <command...>",
		Short: "Run a command in a session (state persists)",
		Args:  cobra.MinimumNArgs(3),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, session := args[0], args[1]
			command := strings.Join(args[2:], " ")
			audit := startAudit("session.exec", host)
			a := app.NewDefault()
			res, aerr := a.SessionExec(cmd.Context(), host, session, command, timeout)
			if aerr != nil {
				audit.fail(aerr)
				// A timeout is reported *with* the state of the session: the caller's
				// next move depends on whether the pane is free or still busy, and a
				// bare failure envelope would not say (§32: partial data on failure).
				if jsonFlag {
					_ = output.Failure("session.exec", host, sessionExecData(res), aerr).Write(os.Stdout)
					exitCode = adapterExitCode(aerr)
				} else {
					fmt.Fprintf(os.Stderr, "rhost: %s: %s", aerr.Code, aerr.Message)
					// Only a timeout needs the extra sentence, because only a timeout
					// leaves the question "is the pane free again?". A refusal like
					// SESSION_BUSY is not a damaged session: something is running in it.
					if aerr.Code == errs.RemoteCommandTimeout {
						sp := "still busy"
						if res.SessionPreserved {
							sp = "usable again"
						}
						fmt.Fprintf(os.Stderr, " (session is %s)", sp)
					}
					fmt.Fprintln(os.Stderr)
					exitCode = adapterExitCode(aerr)
				}
				return nil
			}
			code := res.ExitCode
			audit.succeed("", command, &code)
			if jsonFlag {
				_ = output.Success("session.exec", host, sessionExecData(res)).Write(os.Stdout)
			} else {
				_, _ = os.Stdout.WriteString(res.Output)
				if res.Output != "" && !strings.HasSuffix(res.Output, "\n") {
					fmt.Fprintln(os.Stdout)
				}
			}
			exitCode = res.ExitCode
			return nil
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 60*time.Second, "per-command timeout")
	return cmd
}

// sessionExecView is the `data` of session exec, in success and in failure: the
// fields describe the session as it now is, which is exactly what a caller needs
// when the command did not finish.
type sessionExecView struct {
	SessionID        string `json:"session_id"`
	Output           string `json:"output"`
	ExitCode         int    `json:"exit_code"`
	TimedOut         bool   `json:"timed_out"`
	SessionPreserved bool   `json:"session_preserved"`
}

func sessionExecData(res app.SessionExecResult) sessionExecView {
	return sessionExecView{
		SessionID:        res.SessionID,
		Output:           res.Output,
		ExitCode:         res.ExitCode,
		TimedOut:         res.TimedOut,
		SessionPreserved: res.SessionPreserved,
	}
}

func newSessionSendCmd() *cobra.Command {
	var (
		data string
		key  string
	)
	cmd := &cobra.Command{
		Use:   "send <host> <session> (--data TEXT | --key KEY)",
		Short: "Send raw input or a control key to a session",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, session := args[0], args[1]
			audit := startAudit("session.send", host)
			a := app.NewDefault()
			if aerr := a.SessionSend(cmd.Context(), host, session, data, key, 30*time.Second); aerr != nil {
				audit.fail(aerr)
				emitFailure("session.send", host, aerr)
				return nil
			}
			// The injected data may be sensitive, so record the key but never the data.
			summary := "--data (redacted)"
			if key != "" {
				summary = "key " + key
			}
			audit.succeed("", summary, nil)
			if jsonFlag {
				_ = output.Success("session.send", host, map[string]interface{}{"sent": true}).Write(os.Stdout)
			} else {
				fmt.Println("sent")
			}
			return nil
		},
	}
	cmd.Flags().StringVar(&data, "data", "", "raw text to inject (verbatim)")
	cmd.Flags().StringVar(&key, "key", "", "control key, e.g. C-c, Enter, Up")
	return cmd
}

func newSessionReadCmd() *cobra.Command {
	var (
		since   int
		timeout time.Duration
	)
	cmd := &cobra.Command{
		Use:   "read <host> <session> [--since N]",
		Short: "Read a session's output log incrementally",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, session := args[0], args[1]
			a := app.NewDefault()
			res, aerr := a.SessionRead(cmd.Context(), host, session, since, timeout)
			if aerr != nil {
				emitFailure("session.read", host, aerr)
				return nil
			}
			if jsonFlag {
				data := map[string]interface{}{
					"session_id": res.SessionID,
					"from":       res.From,
					"next":       res.Next,
					"data":       res.Data,
					"more":       res.HasMore,
				}
				_ = output.Success("session.read", host, data).Write(os.Stdout)
			} else {
				_, _ = os.Stdout.WriteString(res.Data)
			}
			return nil
		},
	}
	cmd.Flags().IntVar(&since, "since", 0, "byte offset to read from")
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "read timeout")
	return cmd
}

func newSessionCloseCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "close <host> <session>",
		Short: "Close a session and remove its state",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, session := args[0], args[1]
			audit := startAudit("session.close", host)
			a := app.NewDefault()
			if aerr := a.SessionClose(cmd.Context(), host, session, 30*time.Second); aerr != nil {
				audit.fail(aerr)
				emitFailure("session.close", host, aerr)
				return nil
			}
			audit.succeed("", session, nil)
			if jsonFlag {
				_ = output.Success("session.close", host, map[string]interface{}{"closed": true}).Write(os.Stdout)
			} else {
				fmt.Println("closed")
			}
			return nil
		},
	}
}

func newSessionAttachCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "attach <host> <session>",
		Short: "Attach the local terminal to a session (interactive)",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, session := args[0], args[1]
			a := app.NewDefault()
			if aerr := a.SessionAttach(cmd.Context(), host, session); aerr != nil {
				emitFailure("session.attach", host, aerr)
				return nil
			}
			return nil
		},
	}
}
