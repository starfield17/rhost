// Package cli parses command-line arguments and renders output. It wires the
// application layer to the terminal; it contains no remote-control logic
// (docs/ARCHITECTURE.md).
package cli

import (
	"context"
	"fmt"
	"io"
	"os"
	"os/signal"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/app"
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
	exitCode = 0
	outputFailed = false
	// JSON delivery errors must reach writeEnvelope, including a closed pipe.
	if usageWantsJSON() {
		signal.Ignore(syscall.SIGPIPE)
		defer signal.Reset(syscall.SIGPIPE)
	}
	root := newRootCmd()
	root.SilenceErrors = true
	root.SilenceUsage = true

	if err := root.Execute(); err != nil {
		// Anything reaching here is a usage/flag error: application commands
		// render their own failures and return nil. The usage path still gets an
		// error.code, because agents must never have to parse English text.
		aerr := errs.New(errs.UsageError, err.Error(), false)
		if usageWantsJSON() {
			writeEnvelope(output.Failure("usage", "", nil, aerr))
		} else {
			fmt.Fprintf(os.Stderr, "rhost: %s: %v\n", aerr.Code, err)
		}
		// 255, so that a usage error is never mistaken for a remote status.
		return 255
	}
	if outputFailed {
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
		if a == "--" { // everything after -- is an operand, not our flags
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
		Short: "Run ordinary commands on an SSH-reachable host",
		Long: `rhost runs an ordinary shell command in a remote execution context.

Use rhost exec TARGET --command 'program' for the normal path. rhost orchestrates your
existing OpenSSH configuration and never duplicates authentication or host-key
policy. A target is an alias from ~/.ssh/config, a user@host, or a bare hostname.`,
	}
	root.PersistentFlags().BoolVar(&jsonFlag, "json", false, "emit machine-readable JSON on stdout")
	root.RunE = func(c *cobra.Command, args []string) error {
		if len(args) != 0 {
			return fmt.Errorf("direct execution requires: rhost exec <host> --command <string>")
		}
		if jsonFlag {
			emitFailure("usage", "", errs.New(errs.UsageError, "rhost needs a subcommand", false))
			return nil
		}
		return c.Help()
	}
	root.AddCommand(
		newExecCmd(),
		newTunnelCmd(),
		newDoctorCmd(),
		newHostsCmd(),
		newSessionCmd(),
		newJobCmd(),
		newFsCmd(),
		newAuditCmd(),
		newVersionCmd(),
	)
	return root
}

func runDirectExec(cmd *cobra.Command, host, command, cwd string, envs []string, timeout time.Duration, maxOutput int) error {
	if timeout < 0 || maxOutput < 0 || maxOutput > maxCLIOutputBytes {
		emitFailure("exec", host, errs.New(errs.ConfigInvalid,
			"--timeout must be non-negative and --max-output-bytes between 0 and 67108864", false))
		return nil
	}
	env, err := parseEnv(envs)
	if err != nil {
		emitFailure("exec", host, configErr(err))
		return nil
	}
	if !cmd.Flags().Changed("max-output-bytes") {
		if jsonFlag {
			maxOutput = 1024 * 1024
		} else {
			maxOutput = -1
		}
	}

	ctx, cancel := context.WithCancel(cmd.Context())
	defer cancel()
	sigch := make(chan os.Signal, 1)
	signal.Notify(sigch, os.Interrupt, syscall.SIGTERM)
	defer signal.Stop(sigch)
	signal.Ignore(syscall.SIGPIPE)
	defer signal.Reset(syscall.SIGPIPE)
	var sig atomic.Int32
	go func() {
		select {
		case s := <-sigch:
			if s == os.Interrupt {
				sig.Store(int32(syscall.SIGINT))
			} else {
				sig.Store(int32(syscall.SIGTERM))
			}
			cancel()
		case <-ctx.Done():
		}
	}()

	var stdout, stderr io.Writer = os.Stdout, os.Stderr
	if jsonFlag {
		stdout, stderr = nil, nil
	}
	audit := startAudit("exec", host)
	res, aerr := app.NewDefault().ExecuteStream(ctx, app.StreamExecOptions{
		ExecOptions: app.ExecOptions{Host: host, Command: command, Cwd: cwd, Env: env,
			Timeout: timeout, MaxOutputBytes: maxOutput},
		Stdin: os.Stdin, Stdout: stdout, Stderr: stderr,
	})
	if n := sig.Load(); n != 0 {
		if n == int32(syscall.SIGINT) {
			res.CancelSignal = "SIGINT"
		} else {
			res.CancelSignal = "SIGTERM"
		}
	}
	if aerr != nil {
		audit.fail(aerr)
		if jsonFlag {
			writeEnvelope(output.Failure("exec", host, execData(res), aerr))
		} else {
			fmt.Fprintf(os.Stderr, "rhost: %s: %s\n", aerr.Code, aerr.Message)
		}
		if aerr.Code == errs.RemoteCommandCancelled {
			if res.CancelSignal == "SIGINT" {
				exitCode = 130
			} else {
				exitCode = 143
			}
		} else {
			exitCode = adapterExitCode(aerr)
		}
		return nil
	}
	code := res.ExitCode
	audit.succeed(cwd, command, &code)
	if jsonFlag {
		writeEnvelope(output.Success("exec", host, execData(res)))
	}
	exitCode = code
	return nil
}
