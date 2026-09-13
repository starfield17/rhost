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
	"github.com/starfield17/rhost/internal/shell"
)

const maxCLIOutputBytes = 64 * 1024 * 1024

func newExecCmd() *cobra.Command {
	var (
		cwd       string
		timeout   time.Duration
		envs      []string
		maxOutput int
	)
	cmd := &cobra.Command{
		Use:    "exec <host> [--] <command...>",
		Short:  "Compatibility foreground command; prefer rhost --host",
		Hidden: true,
		Long: `Compatibility surface for existing callers; no new execution features.
For new calls use: rhost --host <host> -- '<command>'

This form retains buffered output, no stdin forwarding, a default 60-second
deadline and a 1 MiB capture limit per stream. --timeout 0 falls back to the
configured default; it does not mean unlimited as it does in the direct form.

Run a command in a fresh remote execution context.

Each call is independent: no shell state, cwd, or environment persists between
exec calls. The command words after the host are joined with spaces and run by a
login bash on the remote host, so shell syntax works:

  rhost exec gpu -- pytest -q
  rhost exec gpu -- 'echo hi | wc -l'

The process exit status mirrors the remote command's exit status. Adapter
failures use 255, timeouts use 124.`,
		Args: cobra.MinimumNArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			host := args[0]
			command := strings.Join(args[1:], " ")
			audit := startAudit("exec", host)

			env, err := parseEnv(envs)
			if err != nil {
				aerr := errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
				audit.fail(aerr)
				emitFailure("exec", host, aerr)
				return nil
			}

			a := app.NewDefault()
			if maxOutput < 0 || maxOutput > maxCLIOutputBytes {
				emitFailure("exec", host, errs.New(errs.ConfigInvalid,
					"max-output-bytes must be between 0 and 67108864", false))
				return nil
			}
			res, aerr := a.Execute(cmd.Context(), app.ExecOptions{
				Host:           host,
				Command:        command,
				Cwd:            cwd,
				Env:            env,
				Timeout:        timeout,
				MaxOutputBytes: maxOutput,
			})
			if aerr != nil {
				audit.fail(aerr)
				renderExecFailure(host, res, aerr)
				return nil
			}
			code := res.ExitCode
			audit.succeed(cwd, command, &code)
			renderExecSuccess(host, res)
			return nil
		},
	}
	cmd.Flags().StringVar(&cwd, "cwd", "", "working directory on the remote host")
	cmd.Flags().IntVar(&maxOutput, "max-output-bytes", 1024*1024, "maximum bytes per stream (0 = unlimited)")
	cmd.Flags().DurationVar(&timeout, "timeout", 60*time.Second, "foreground timeout (e.g. 30s, 2m)")
	cmd.Flags().StringArrayVar(&envs, "env", nil, "environment variable KEY=VALUE (repeatable)")
	return cmd
}

// execView is the `data` of one exec result. It is a
// struct rather than a map so the field set is declared once, in the order it is
// meant to be read: what the command returned, then how much of it was lost.
//
// The byte counts are the *true* totals the remote produced, so `stdout` being
// shorter than `stdout_bytes` is what `stdout_truncated` means; and
// `cleanup_confirmed` is false whenever a timed-out command could not be shown to
// have stopped, which is a different fact from "it did not run".
type execView struct {
	ExitCode         int    `json:"exit_code"`
	Stdout           string `json:"stdout"`
	Stderr           string `json:"stderr"`
	TimedOut         bool   `json:"timed_out"`
	Cancelled        bool   `json:"cancelled"`
	CancelSignal     string `json:"cancel_signal,omitempty"`
	CleanupConfirmed bool   `json:"cleanup_confirmed"`
	StdoutTruncated  bool   `json:"stdout_truncated"`
	StderrTruncated  bool   `json:"stderr_truncated"`
	StdoutBytes      int64  `json:"stdout_bytes"`
	StderrBytes      int64  `json:"stderr_bytes"`
	DurationMS       int64  `json:"duration_ms"`
}

func execData(res app.ExecResult) execView {
	return execView{
		ExitCode:         res.ExitCode,
		Stdout:           res.Stdout,
		Stderr:           res.Stderr,
		TimedOut:         res.TimedOut,
		Cancelled:        res.Cancelled,
		CancelSignal:     res.CancelSignal,
		CleanupConfirmed: res.CleanupConfirmed,
		StdoutTruncated:  res.StdoutTruncated,
		StderrTruncated:  res.StderrTruncated,
		StdoutBytes:      res.StdoutBytes,
		StderrBytes:      res.StderrBytes,
		DurationMS:       res.Duration.Milliseconds(),
	}
}

func renderExecSuccess(host string, res app.ExecResult) {
	if jsonFlag {
		writeEnvelope(output.Success("exec", host, execData(res)))
	} else {
		_, _ = os.Stdout.WriteString(res.Stdout)
		_, _ = os.Stderr.WriteString(res.Stderr)
		noteTruncated(res)
	}
	exitCode = res.ExitCode
}

func renderExecFailure(host string, res app.ExecResult, aerr *errs.Error) {
	if jsonFlag {
		writeEnvelope(output.Failure("exec", host, execData(res), aerr))
	} else {
		fmt.Fprintf(os.Stderr, "rhost: %s: %s\n", aerr.Code, aerr.Message)
		if res.Stdout != "" {
			_, _ = os.Stdout.WriteString(res.Stdout)
		}
		if res.Stderr != "" {
			_, _ = os.Stderr.WriteString(res.Stderr)
		}
		noteTruncated(res)
		if res.TimedOut && !res.CleanupConfirmed {
			fmt.Fprintln(os.Stderr, "warning: the remote command may still be running; the process group could not be confirmed dead")
		}
	}
	exitCode = adapterExitCode(aerr)
}

// noteTruncated tells a human that the text they are reading is not all of it.
// In JSON the same fact is two booleans and two counters.
func noteTruncated(res app.ExecResult) {
	if res.StdoutTruncated || res.StderrTruncated {
		fmt.Fprintf(os.Stderr, "rhost: output truncated (stdout %d bytes, stderr %d bytes total)\n",
			res.StdoutBytes, res.StderrBytes)
	}
}

// parseEnv parses repeated --env KEY=VALUE flags, validating each key.
func parseEnv(pairs []string) (map[string]string, error) {
	if len(pairs) == 0 {
		return nil, nil
	}
	m := make(map[string]string, len(pairs))
	for _, p := range pairs {
		i := strings.IndexByte(p, '=')
		if i <= 0 {
			return nil, fmt.Errorf("invalid --env %q (want KEY=VALUE)", p)
		}
		key, value := p[:i], p[i+1:]
		if err := shell.ValidateEnvKey(key); err != nil {
			return nil, err
		}
		m[key] = value
	}
	return m, nil
}
