package cli

import (
	"fmt"
	"os"
	"time"

	"github.com/spf13/cobra"
	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/fileops"
	"github.com/starfield17/rhost/internal/output"
)

// newFsCmd builds `rhost fs put|get|sync` (docs/architecture/files-and-json.md).
//
// The endpoints stay separate arguments rather than scp's `host:path` shorthand:
// a positional `~/x` is ambiguous, and guessing wrong here copies a file the wrong
// way. rhost never parses the host part — OpenSSH resolves it (§5).
func newFsCmd() *cobra.Command {
	cmd := newGroup("fs", "Move files between this machine and a remote host", `Copy single files with put and get (scp), and directories with sync
(rsync).

Nothing is ever deleted by a sync unless --delete is given, and --dry-run
reports the plan without touching either side. Read the plan before you sync
into a directory you did not create.

Paths follow scp's convention: a trailing slash on a synced directory means its
contents, not the directory itself. rhost normalises it, so it is accepted
either way.`)
	cmd.AddCommand(newFsPutCmd(), newFsGetCmd(), newFsSyncCmd())
	cmd.AddCommand(newFsMirrorCmd(), newFsBatchCmd())
	for _, op := range []string{"read", "write", "patch"} {
		cmd.AddCommand(newRemoteFileCmd(op))
	}
	return cmd
}

func fsTimeoutFlags(cmd *cobra.Command, timeout *time.Duration) {
	cmd.Flags().DurationVar(timeout, "timeout", 5*time.Minute,
		"maximum time for the transfer itself (e.g. 30s, 10m)")
}

func newFsPutCmd() *cobra.Command {
	var timeout time.Duration
	var resume, checksum, parents bool
	cmd := &cobra.Command{
		Use:   "put <host> <local-path> <remote-path>",
		Short: "Copy one local file to the remote host",
		Args:  cobra.ExactArgs(3),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, local, remote := args[0], args[1], args[2]
			audit := startAudit("fs.put", host)
			a := app.NewDefault()
			res, aerr := a.FsPut(cmd.Context(), app.FsPutOptions{
				Host: host, LocalPath: local, Remote: remote, Timeout: timeout,
				Resume: resume, Checksum: checksum, Parents: parents,
			})
			if aerr != nil {
				audit.fail(aerr)
				emitFailure("fs.put", host, aerr)
				return nil
			}
			audit.succeed("", "put "+local+" -> "+remote, nil)
			if jsonFlag {
				writeEnvelope(output.Success("fs.put", host, res))
			} else {
				renderFsTransfer(res)
			}
			return nil
		},
	}
	fsTimeoutFlags(cmd, &timeout)
	cmd.Flags().BoolVar(&resume, "resume", false, "resume using rsync partial files and verify SHA-256")
	cmd.Flags().BoolVar(&checksum, "checksum", false, "verify end-to-end SHA-256 using rsync and sha256sum")
	cmd.Flags().BoolVar(&parents, "parents", false, "create missing remote parent directories")
	return cmd
}

func newFsGetCmd() *cobra.Command {
	var timeout time.Duration
	var resume, checksum bool
	cmd := &cobra.Command{
		Use:   "get <host> <remote-path> <local-path>",
		Short: "Copy one remote file to this machine",
		Args:  cobra.ExactArgs(3),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, remote, local := args[0], args[1], args[2]
			audit := startAudit("fs.get", host)
			a := app.NewDefault()
			res, aerr := a.FsGet(cmd.Context(), app.FsGetOptions{
				Host: host, Remote: remote, LocalPath: local, Timeout: timeout,
				Resume: resume, Checksum: checksum,
			})
			if aerr != nil {
				audit.fail(aerr)
				emitFailure("fs.get", host, aerr)
				return nil
			}
			audit.succeed("", "get "+remote+" -> "+local, nil)
			if jsonFlag {
				writeEnvelope(output.Success("fs.get", host, res))
			} else {
				renderFsTransfer(res)
			}
			return nil
		},
	}
	fsTimeoutFlags(cmd, &timeout)
	cmd.Flags().BoolVar(&resume, "resume", false, "resume using rsync partial files and verify SHA-256")
	cmd.Flags().BoolVar(&checksum, "checksum", false, "verify end-to-end SHA-256 using rsync and sha256sum")
	return cmd
}

func newFsSyncCmd() *cobra.Command {
	var (
		doDelete bool
		dryRun   bool
		checksum bool
		excludes []string
		timeout  time.Duration
	)
	cmd := &cobra.Command{
		Use:   "sync <host> <local-dir> <remote-dir>",
		Short: "Bring a remote directory in line with a local one",
		Long: `Copy a directory tree to the remote host with rsync.

Nothing is deleted unless --delete is given. --dry-run prints the plan and
changes nothing; an empty plan means the two sides already match.

Syncing into a top-level directory (/, /srv, ~) with --delete is refused: the
prune would be a disaster, and there is no flag that makes it safe.`,
		Args: cobra.ExactArgs(3),
		RunE: func(cmd *cobra.Command, args []string) error {
			host, local, remote := args[0], args[1], args[2]
			audit := startAudit("fs.sync", host)
			a := app.NewDefault()
			res, aerr := a.FsSync(cmd.Context(), app.FsSyncOptions{
				Host: host, LocalPath: local, Remote: remote,
				Delete: doDelete, DryRun: dryRun, Excludes: excludes, Timeout: timeout,
				Checksum: checksum,
			})
			if aerr != nil {
				audit.fail(aerr)
				emitFailure("fs.sync", host, aerr)
				return nil
			}
			summary := "sync " + local + " -> " + remote
			if doDelete {
				summary += " --delete"
			}
			if dryRun {
				summary += " --dry-run"
			}
			audit.succeed("", summary, nil)
			if jsonFlag {
				writeEnvelope(output.Success("fs.sync", host, res))
			} else {
				renderFsSync(res)
			}
			return nil
		},
	}
	cmd.Flags().BoolVar(&doDelete, "delete", false,
		"remove remote files that do not exist locally (destructive)")
	cmd.Flags().BoolVar(&checksum, "checksum", false, "compare content checksums")
	cmd.Flags().BoolVar(&dryRun, "dry-run", false,
		"report what would change without copying anything")
	cmd.Flags().StringArrayVar(&excludes, "exclude", nil,
		"pattern to skip, rsync syntax (repeatable), e.g. --exclude .git")
	fsTimeoutFlags(cmd, &timeout)
	return cmd
}

// renderFsTransfer prints the effective endpoints, not the arguments as typed:
// the destination of a `get` into an existing directory is a file inside it, and
// the JSON says the same thing.
func renderFsTransfer(res app.FsTransferResult) {
	fmt.Printf("transferred %s via %s\n", humanBytes(res.Size), res.Backend)
	fmt.Printf("  %s\n", res.Source)
	fmt.Printf("  -> %s\n", res.Destination)
	if res.ChecksumVerified {
		// Say so out loud: the whole point of asking for a verified copy is to see
		// that the two ends agree, and silence would read as "not verified".
		fmt.Fprintln(os.Stderr, "  verified: identical SHA-256 at both ends")
	}
	if res.ResumeEnabled {
		fmt.Fprintln(os.Stderr, "  resumable: rsync partial files in .rhost-partial")
	}
}

// renderFsSync prints exactly the plan the JSON carries (§28: agent JSON and
// human preview stay consistent), one action per line.
func renderFsSync(res app.FsSyncResult) {
	renderSyncDirection(res, "synced", "would sync (dry run, nothing copied)")
}

// renderSyncDirection is renderFsSync with the verbs supplied, because `mirror`
// reuses both the result type and the plan format, and a person reading
// "synced" while downloading a directory would be entitled to wonder which
// direction ran.
func renderSyncDirection(res app.FsSyncResult, applied, preview string) {
	head := applied
	if res.DryRun {
		head = preview
	}
	fmt.Printf("%s via %s%s\n", head, res.Backend, deleteNote(res.Delete))
	fmt.Printf("  %s -> %s\n", res.Source, res.Destination)
	for _, ch := range res.Changes {
		fmt.Printf("  %s %s\n", changeSymbol(ch.Action), ch.Path)
	}
	for _, line := range res.Notes {
		fmt.Printf("  ! %s\n", line)
	}
	switch {
	case len(res.Changes) == 0:
		fmt.Fprintln(os.Stderr, "no files changed")
	default:
		fmt.Fprintf(os.Stderr, "%d file action(s), %d directory action(s), %d deletion(s)\n",
			res.Files, res.Directories, res.Deletes)
	}
	if !res.Multiplexed {
		fmt.Fprintln(os.Stderr, "note: rsync could not reuse the multiplexed connection and opened its own")
	}
}

func deleteNote(delete bool) string {
	if delete {
		return " (with --delete enabled)"
	}
	return ""
}

func changeSymbol(action fileops.Action) string {
	switch action {
	case fileops.ActionDelete:
		return "-"
	case fileops.ActionDirectory:
		return "d"
	case fileops.ActionSkip:
		return "s"
	default:
		return "+"
	}
}

// humanBytes is display-only; the JSON keeps the integer byte count.
func humanBytes(n int64) string {
	if n < 1024 {
		return fmt.Sprintf("%d B", n)
	}
	units := []string{"KiB", "MiB", "GiB", "TiB"}
	value := float64(n)
	i := -1
	for value >= 1024 && i < len(units)-1 {
		value /= 1024
		i++
	}
	return fmt.Sprintf("%.1f %s", value, units[i])
}
