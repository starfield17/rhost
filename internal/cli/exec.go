package cli

import (
	"fmt"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/app"
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
	command := &shellCommandValue{}
	cmd := &cobra.Command{
		Use:   "exec <host> --command <shell-program>",
		Short: "Run a shell program on a remote host",
		Long: `Run one exact shell program in a fresh remote execution context.

Each call is independent: no shell state, cwd, or environment persists between
exec calls. The --command value runs under a remote login bash, so pipes,
redirections, variables and compound shell syntax work:

  rhost exec gpu --command 'pytest -q'
  rhost exec gpu --command 'echo hi | wc -l'

Human mode streams stdout and stderr and forwards stdin. There is no default
execution deadline. The process status mirrors the remote command; adapter
failures use 255 and timeouts use 124.`,
		Args: command.validate("<host>"),
		RunE: func(cmd *cobra.Command, args []string) error {
			return runDirectExec(cmd, args[0], command.value, cwd, envs, timeout, maxOutput)
		},
	}
	bindShellCommand(cmd, command)
	cmd.Flags().StringVar(&cwd, "cwd", "", "working directory on the remote host")
	cmd.Flags().IntVar(&maxOutput, "max-output-bytes", 0, "captured bytes per stream (0 = unlimited; JSON defaults to 1 MiB)")
	cmd.Flags().DurationVar(&timeout, "timeout", 0, "execution deadline; 0 waits until completion")
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
