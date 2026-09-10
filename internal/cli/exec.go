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

func newExecCmd() *cobra.Command {
	var (
		cwd     string
		timeout time.Duration
		envs    []string
	)
	cmd := &cobra.Command{
		Use:   "exec <host> [--] <command...>",
		Short: "Run a command on a remote host (stateless foreground)",
		Long: `Run a command in a fresh remote execution context.

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

			env, err := parseEnv(envs)
			if err != nil {
				emitFailure("exec", host, errs.Wrap(errs.ConfigInvalid, err.Error(), false, err), 255)
				return nil
			}

			a := app.NewDefault()
			res, aerr := a.Execute(cmd.Context(), app.ExecOptions{
				Host:    host,
				Command: command,
				Cwd:     cwd,
				Env:     env,
				Timeout: timeout,
			})
			if aerr != nil {
				renderExecFailure(host, res, aerr)
				return nil
			}
			renderExecSuccess(host, res)
			return nil
		},
	}
	cmd.Flags().StringVar(&cwd, "cwd", "", "working directory on the remote host")
	cmd.Flags().DurationVar(&timeout, "timeout", 60*time.Second, "foreground timeout (e.g. 30s, 2m)")
	cmd.Flags().StringArrayVar(&envs, "env", nil, "environment variable KEY=VALUE (repeatable)")
	return cmd
}

func execData(res app.ExecResult) map[string]interface{} {
	return map[string]interface{}{
		"exit_code":   res.ExitCode,
		"stdout":      res.Stdout,
		"stderr":      res.Stderr,
		"timed_out":   res.TimedOut,
		"duration_ms": res.Duration.Milliseconds(),
	}
}

func renderExecSuccess(host string, res app.ExecResult) {
	if jsonFlag {
		_ = output.Success("exec", host, execData(res)).Write(os.Stdout)
	} else {
		_, _ = os.Stdout.WriteString(res.Stdout)
		_, _ = os.Stderr.WriteString(res.Stderr)
	}
	exitCode = res.ExitCode
}

func renderExecFailure(host string, res app.ExecResult, aerr *errs.Error) {
	if jsonFlag {
		_ = output.Failure("exec", host, execData(res), aerr).Write(os.Stdout)
	} else {
		fmt.Fprintf(os.Stderr, "rhost: %s: %s\n", aerr.Code, aerr.Message)
		if res.Stdout != "" {
			_, _ = os.Stdout.WriteString(res.Stdout)
		}
		if res.Stderr != "" {
			_, _ = os.Stderr.WriteString(res.Stderr)
		}
	}
	if aerr.Code == errs.RemoteCommandTimeout {
		exitCode = 124
	} else {
		exitCode = 255
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
