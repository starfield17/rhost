package cli

import (
	"context"
	"fmt"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/spf13/cobra"
	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

// execManyResult is one target's row in the aggregate report. It embeds the same
// view `exec` returns, so a host that failed for a transport reason still says
// which, and a host that ran says what it exited with.
//
// `skipped` marks a target that never started because --stop-on-error fired;
// rhost stops *scheduling*, it never abandons or kills a command already running.
type execManyResult struct {
	execView
	Host    string               `json:"host"`
	OK      bool                 `json:"ok"`
	Skipped bool                 `json:"skipped,omitempty"`
	Error   *output.ErrorPayload `json:"error,omitempty"`
}

// execManyReport keeps the rows in the order the flags were given. The aggregate
// envelope is `ok: true` whenever the fan-out itself completed: partial failures
// are inside, per target, which is the whole point of running several hosts.
type execManyReport struct {
	Results   []execManyResult `json:"results"`
	Succeeded int              `json:"succeeded"`
	Failed    int              `json:"failed"`
	Skipped   int              `json:"skipped"`
}

func newExecManyCmd() *cobra.Command {
	var (
		hosts, envs    []string
		cwd            string
		parallel       int
		maxOutput      int
		serial         bool
		stopOnError    bool
		delay, timeout time.Duration
	)
	c := &cobra.Command{
		Use:   "exec-many --host <host> [--host <host>] -- <command...>",
		Short: "Run one command on several SSH targets",
		Long: `Run one command on several targets, in the order they were given.

Each target runs through the same exec path as ` + "`rhost exec`" + ` — its own OpenSSH
invocation over the shared ControlMaster — and reports its own exit status,
output and error. Nothing is retried and nothing is persisted: this is foreground
fan-out, not a scheduler, so a CLI that exits stops the run.

Process status is aggregate: 0 when every command exited 0, 1 when a remote
command failed or a target was skipped, 255 when a target could not be reached at
all. Read the per-target rows, not just the status.`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(c *cobra.Command, args []string) error {
			if len(hosts) == 0 {
				emitFailure("exec-many", "", errs.New(errs.UsageError,
					"at least one --host is required", false))
				return nil
			}
			if parallel < 1 || delay < 0 || maxOutput < 0 || maxOutput > maxCLIOutputBytes {
				emitFailure("exec-many", "", errs.New(errs.ConfigInvalid,
					"--parallel must be positive, --delay non-negative, and --max-output-bytes between 0 and 67108864", false))
				return nil
			}
			env, err := parseEnv(envs)
			if err != nil {
				emitFailure("exec-many", "", configErr(err))
				return nil
			}
			if serial {
				parallel = 1
			}
			report := runMany(c.Context(), execManyPlan{
				hosts: hosts, parallel: parallel, delay: delay, stopOnError: stopOnError,
				command: strings.Join(args, " "), cwd: cwd, env: env,
				timeout: timeout, maxOutput: maxOutput,
			})
			renderExecMany(report)
			return nil
		},
	}
	c.Flags().StringArrayVar(&hosts, "host", nil, "target to run on (repeatable)")
	c.Flags().IntVar(&parallel, "parallel", 4, "maximum targets in flight")
	c.Flags().BoolVar(&serial, "serial", false, "run one target at a time (same as --parallel 1)")
	c.Flags().DurationVar(&delay, "delay", 0, "each worker waits this long between targets")
	c.Flags().BoolVar(&stopOnError, "stop-on-error", false,
		"do not start further targets after one fails; running targets finish")
	c.Flags().DurationVar(&timeout, "timeout", time.Minute, "deadline per target")
	c.Flags().IntVar(&maxOutput, "max-output-bytes", 1024*1024,
		"maximum bytes per stream, per target (0 = unlimited)")
	c.Flags().StringVar(&cwd, "cwd", "", "working directory on each remote host")
	c.Flags().StringArrayVar(&envs, "env", nil, "environment variable KEY=VALUE (repeatable)")
	return c
}

// execManyPlan is one fan-out, assembled from the flags.
type execManyPlan struct {
	hosts       []string
	parallel    int
	delay       time.Duration
	stopOnError bool
	command     string
	cwd         string
	env         map[string]string
	timeout     time.Duration
	maxOutput   int
}

// runMany walks the target list with a fixed number of workers.
//
// Workers take the *next* index rather than a slice of the list, so --parallel 2
// over eight hosts starts hosts 3 and 4 as soon as either of the first two
// returns; and results are written to their own index, so the report order is the
// order the flags were given, never the order of completion.
func runMany(ctx context.Context, p execManyPlan) execManyReport {
	hosts := p.hosts
	results := make([]execManyResult, len(hosts))
	for i, host := range hosts {
		results[i] = execManyResult{Host: host, Skipped: true,
			execView: execView{ExitCode: -1}}
	}
	var (
		next   int
		mu     sync.Mutex
		failed atomic.Bool
		wg     sync.WaitGroup
	)
	for w := 0; w < p.parallel; w++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				mu.Lock()
				switch {
				case next >= len(hosts), ctx.Err() != nil:
					mu.Unlock()
					return
				case p.stopOnError && failed.Load():
					// Everything not yet started is left untouched; the rows keep
					// their skipped marker instead of pretending to be a result.
					mu.Unlock()
					return
				}
				i := next
				next++
				mu.Unlock()

				host := hosts[i]
				audit := startAudit("exec-many", host)
				res, aerr := app.NewDefault().Execute(ctx, app.ExecOptions{
					Host: host, Command: p.command, Cwd: p.cwd, Env: p.env,
					Timeout: p.timeout, MaxOutputBytes: p.maxOutput,
				})
				row := execManyResult{Host: host, execView: execData(res), Error: output.ErrorOf(aerr)}
				row.OK = aerr == nil
				if aerr != nil {
					failed.Store(true)
					audit.fail(aerr)
				} else {
					if res.ExitCode != 0 {
						failed.Store(true)
					}
					code := res.ExitCode
					audit.succeed(p.cwd, p.command, &code)
				}
				results[i] = row

				if p.delay > 0 {
					select {
					case <-ctx.Done():
						return
					case <-time.After(p.delay):
					}
				}
			}
		}()
	}
	wg.Wait()

	report := execManyReport{Results: results}
	for _, row := range results {
		switch {
		case row.Skipped:
			report.Skipped++
		case row.OK && row.ExitCode == 0:
			report.Succeeded++
		default:
			report.Failed++
		}
	}
	return report
}

// renderExecMany prints one section per target, in the order they were given, in
// the `for f in $list; do echo "==> $f"` idiom people already recognise from
// `grep` over several files. The JSON carries the same rows with the same fields.
func renderExecMany(report execManyReport) {
	if jsonFlag {
		_ = output.Success("exec-many", "", report).Write(os.Stdout)
	} else {
		for _, row := range report.Results {
			switch {
			case row.Skipped:
				fmt.Printf("==> %s <skipped>\n", row.Host)
			case !row.OK:
				fmt.Printf("==> %s <error: %s: %s>\n", row.Host, row.Error.Code, row.Error.Message)
			default:
				fmt.Printf("==> %s (exit %d)\n", row.Host, row.ExitCode)
				os.Stdout.WriteString(row.Stdout)
				os.Stderr.WriteString(row.Stderr)
			}
		}
		fmt.Fprintf(os.Stderr, "%d ok, %d failed, %d skipped\n",
			report.Succeeded, report.Failed, report.Skipped)
	}
	exitCode = aggregateExitCode(report)
}

// aggregateExitCode is documented on the command: 0 for an all-green run, 1 when
// a remote command failed or a target never started, 255 when a target could not
// be reached. A 1 still means "a status the remote produced", so nothing here
// contradicts the single-host exit-code policy.
func aggregateExitCode(report execManyReport) int {
	adapter := false
	for _, row := range report.Results {
		if !row.OK && !row.Skipped {
			adapter = true
			break
		}
	}
	switch {
	case adapter:
		return 255
	case report.Failed > 0 || report.Skipped > 0:
		return 1
	default:
		return 0
	}
}
