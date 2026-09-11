package cli

import (
	"context"
	"fmt"
	"io"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/spf13/cobra"

	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

func newWatchCmd() *cobra.Command {
	var (
		interval time.Duration
		count    int
		timeout  time.Duration
	)
	cmd := &cobra.Command{
		Use:   "watch <host>",
		Short: "Live-refresh a host's status until interrupted",
		Long: `Repeatedly take the same snapshot ` + "`rhost status`" + ` returns, until
interrupted (Ctrl-C) or --count refreshes have been printed.

watch owns no state. Each refresh re-reads the host and rediscovers its sessions
and jobs remotely, so a dropped connection shows as offline and the next
successful refresh reconstructs the truth from the host rather than from local
memory (docs/ARCHITECTURE.md §31).

With --json it prints one envelope per refresh, one per line (NDJSON): each line
is a complete result document, so a consumer can parse the stream incrementally.`,
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			// Ctrl-C cancels the loop and lets the in-flight refresh stop, rather
			// than killing the process mid-write.
			ctx, stop := signal.NotifyContext(cmd.Context(), os.Interrupt, syscall.SIGTERM)
			defer stop()
			return runWatch(ctx, args[0], interval, count, timeout)
		},
	}
	cmd.Flags().DurationVar(&interval, "interval", 2*time.Second, "time between refreshes")
	cmd.Flags().IntVar(&count, "count", 0, "stop after this many refreshes (0 = until interrupted)")
	cmd.Flags().DurationVar(&timeout, "timeout", 30*time.Second, "per-refresh probe timeout")
	return cmd
}

func runWatch(ctx context.Context, host string, interval time.Duration, count int, timeout time.Duration) error {
	if interval <= 0 {
		interval = 2 * time.Second
	}
	a := app.NewDefault()

	for refreshes := 1; ; refreshes++ {
		res, aerr := a.Status(ctx, app.StatusOptions{Host: host, Timeout: timeout})
		if ctx.Err() != nil {
			return nil // interrupted mid-refresh: do not report a spurious offline tick
		}
		if aerr != nil {
			res = offlineStatus(aerr)
		}

		if jsonFlag {
			_ = output.Success("watch", host, res).Write(os.Stdout)
		} else {
			if isTerminal(os.Stdout) {
				clearScreen(os.Stdout)
			}
			renderStatus(os.Stdout, host, res)
			fmt.Fprintf(os.Stdout, "\nrefreshed %s\n", res.ProbedAt)
		}

		if count > 0 && refreshes >= count {
			return nil
		}
		select {
		case <-ctx.Done():
			return nil
		case <-time.After(interval):
		}
	}
}

// offlineStatus is one unreachable refresh. It is a normal observation for watch,
// so it is a successful envelope carrying online=false rather than an error: the
// monitor is working; the host is not reachable.
func offlineStatus(aerr *errs.Error) app.StatusResult {
	return app.StatusResult{
		Online:         false,
		ProbedAt:       time.Now().UTC().Format(time.RFC3339),
		OfflineCode:    string(aerr.Code),
		OfflineMessage: aerr.Message,
		Unavailable:    []string{},
	}
}

// isTerminal reports whether w is an interactive terminal, so watch only clears
// the screen for a human instead of emitting escape bytes into a pipe or a log.
func isTerminal(w io.Writer) bool {
	f, ok := w.(*os.File)
	if !ok {
		return false
	}
	fi, err := f.Stat()
	if err != nil {
		return false
	}
	return fi.Mode()&os.ModeCharDevice != 0
}

func clearScreen(w io.Writer) {
	fmt.Fprint(w, "\x1b[2J\x1b[H")
}
