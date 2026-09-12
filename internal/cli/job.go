package cli

import (
	"encoding/base64"
	"fmt"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/output"
)

func newJobCmd() *cobra.Command {
	cmd := newGroup("job", "Manage detached background jobs", `A job is a command that keeps running after the CLI exits and SSH
disconnects. It is owned by remote files and processes — never by rhost — so it
survives the local process and is rediscoverable at any time.

A job is the right tool when work will outlive a foreground timeout. For
anything that finishes quickly, prefer `+"`rhost exec`"+`; for stateful
interactive work, prefer a session.

Address a job by the id that `+"`job start`"+` printed (`+"`j_`"+` plus 12
hex digits). A --name works as well for status, logs, stop and kill, but only
while it matches exactly one job: names may collide, and a collision is an
error rather than a guess. A --name must not look like a generated id.

A job is only ever reported as running, and only ever signalled, when its
process identity is verified against the boot id and process start time recorded
at launch. If the recorded pid now belongs to another process — or cannot be
verified at all — the job is stale, nothing is signalled, and `+"`signalled`"+`
in the JSON says so.`)
	cmd.AddCommand(
		newJobStartCmd(),
		newJobListCmd(),
		newJobStatusCmd(),
		newJobLogsCmd(),
		newJobStopCmd(),
		newJobKillCmd(),
	)
	return cmd
}

func newJobStartCmd() *cobra.Command {
	var (
		name    string
		cwd     string
		envs    []string
		timeout time.Duration
	)
	cmd := &cobra.Command{
		Use:   "start <host> [flags] -- <command...>",
		Short: "Start a detached background job",
		Args:  cobra.MinimumNArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			host := args[0]
			command := strings.Join(args[1:], " ")
			audit := startAudit("job.start", host)
			env, err := parseEnv(envs)
			if err != nil {
				aerr := configErr(err)
				audit.fail(aerr)
				emitFailure("job.start", host, aerr)
				return nil
			}
			a := app.NewDefault()
			res, aerr := a.JobStart(cmd.Context(), app.JobStartOptions{
				Host:    host,
				Name:    name,
				Cwd:     cwd,
				Command: command,
				Env:     env,
				Timeout: timeout,
			})
			if aerr != nil {
				audit.fail(aerr)
				if jsonFlag {
					_ = output.Failure("job.start", host, res, aerr).Write(os.Stdout)
					exitCode = adapterExitCode(aerr)
				} else {
					fmt.Fprintf(os.Stderr, "rhost: %s: %s (job id %s; state unknown, query before retrying)\n",
						aerr.Code, aerr.Message, res.ID)
					exitCode = adapterExitCode(aerr)
				}
				return nil
			}
			audit.succeed(cwd, command, nil)
			if jsonFlag {
				_ = output.Success("job.start", host, res).Write(os.Stdout)
			} else {
				fmt.Printf("started job %s (pid %d, state %s)\n", res.ID, res.PID, res.State)
			}
			return nil
		},
	}
	cmd.Flags().StringVar(&name, "name", "", "human-friendly label, addressable while unique (must not look like a job id)")
	cmd.Flags().StringVar(&cwd, "cwd", "", "working directory on the remote host")
	cmd.Flags().StringArrayVar(&envs, "env", nil, "environment variable KEY=VALUE (repeatable)")
	cmd.Flags().DurationVar(&timeout, "timeout", 60*time.Second, "launch timeout")
	return cmd
}

func newJobListCmd() *cobra.Command {
	var timeout time.Duration
	cmd := &cobra.Command{
		Use:   "list <host>",
		Short: "List jobs and their states",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			host := args[0]
			a := app.NewDefault()
			jobs, aerr := a.JobList(cmd.Context(), host, timeout)
			if aerr != nil {
				emitFailure("job.list", host, aerr)
				return nil
			}
			if jsonFlag {
				_ = output.Success("job.list", host, map[string]interface{}{"jobs": jobs}).Write(os.Stdout)
				return nil
			}
			if len(jobs) == 0 {
				fmt.Fprintln(os.Stderr, "no jobs")
				return nil
			}
			fmt.Printf("%-16s %-16s %-8s %-9s %s\n", "ID", "NAME", "STATE", "EXIT", "COMMAND")
			for _, j := range jobs {
				ec := "-"
				if j.ExitCode >= 0 {
					ec = strconv.Itoa(j.ExitCode)
				}
				fmt.Printf("%-16s %-16s %-8s %-9s %s\n", j.ID, j.Name, j.State, ec, j.Command)
			}
			return nil
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "list timeout")
	return cmd
}

func newJobStatusCmd() *cobra.Command {
	var timeout time.Duration
	cmd := &cobra.Command{
		Use:   "status <host> <job-id-or-name>",
		Short: "Show a job's state and exit code",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, id := args[0], args[1]
			a := app.NewDefault()
			info, aerr := a.JobStatus(cmd.Context(), host, id, timeout)
			if aerr != nil {
				emitFailure("job.status", host, aerr)
				return nil
			}
			if jsonFlag {
				_ = output.Success("job.status", host, info).Write(os.Stdout)
			} else {
				// -1 means "no exit code recorded yet", which reads as a lie when
				// printed as a number; the JSON keeps the integer.
				exitField := "-"
				if info.ExitCode >= 0 {
					exitField = strconv.Itoa(info.ExitCode)
				}
				fmt.Printf("%s  state=%s pid=%d exit=%s\n", info.ID, info.State, info.PID, exitField)
				if info.Command != "" {
					fmt.Printf("command=%s\n", info.Command)
				}
				if info.StartedAt != "" {
					fmt.Printf("started_at=%s\n", info.StartedAt)
				}
				if info.FinishedAt != "" {
					fmt.Printf("finished_at=%s\n", info.FinishedAt)
				}
			}
			return nil
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "status timeout")
	return cmd
}

func newJobLogsCmd() *cobra.Command {
	var (
		stream  string
		since   int
		timeout time.Duration
	)
	cmd := &cobra.Command{
		Use:   "logs <host> <job-id-or-name> [--stream stdout|stderr] [--since N]",
		Short: "Read a job's stdout/stderr incrementally",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, id := args[0], args[1]
			a := app.NewDefault()
			res, aerr := a.JobLogs(cmd.Context(), host, id, stream, since, timeout)
			if aerr != nil {
				emitFailure("job.logs", host, aerr)
				return nil
			}
			if jsonFlag {
				_ = output.Success("job.logs", host, res).Write(os.Stdout)
				return nil
			}
			data, derr := base64.StdEncoding.DecodeString(res.Data)
			if derr != nil {
				emitFailure("job.logs", host, configErr(derr))
				return nil
			}
			if _, err := os.Stdout.Write(data); err != nil {
				return err
			}
			return nil
		},
	}
	cmd.Flags().StringVar(&stream, "stream", "stdout", "log stream: stdout or stderr")
	cmd.Flags().IntVar(&since, "since", 0, "byte offset to read from")
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "read timeout")
	return cmd
}

func newJobStopCmd() *cobra.Command {
	var timeout time.Duration
	cmd := &cobra.Command{
		Use:   "stop <host> <job-id-or-name>",
		Short: "Stop a job (SIGTERM to its process group)",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			return runJobSignal("job.stop", cmd, args[0], args[1], "TERM", timeout)
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "signal timeout")
	return cmd
}

func newJobKillCmd() *cobra.Command {
	var timeout time.Duration
	cmd := &cobra.Command{
		Use:   "kill <host> <job-id-or-name>",
		Short: "Kill a job (SIGKILL to its process group)",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			return runJobSignal("job.kill", cmd, args[0], args[1], "KILL", timeout)
		},
	}
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "signal timeout")
	return cmd
}

func runJobSignal(op string, cmd *cobra.Command, host, id, signal string, timeout time.Duration) error {
	audit := startAudit(op, host)
	a := app.NewDefault()
	res, aerr := a.JobSignal(cmd.Context(), host, id, signal, timeout)
	if aerr != nil {
		audit.fail(aerr)
		emitFailure(op, host, aerr)
		return nil
	}
	audit.succeed("", id, nil)
	if jsonFlag {
		_ = output.Success(op, host, res).Write(os.Stdout)
	} else if !res.Signalled {
		if res.State == "stale" {
			fmt.Printf("%s: no signal sent (process identity not verified), state=%s\n", res.JobID, res.State)
		} else {
			fmt.Printf("%s: no signal needed (job already final), state=%s\n", res.JobID, res.State)
		}
	} else {
		fmt.Printf("%s: sent %s, state=%s\n", res.JobID, res.Signal, res.State)
	}
	return nil
}
