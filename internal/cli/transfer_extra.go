package cli

import (
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/spf13/cobra"
	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

// newFsMirrorCmd downloads a remote directory.
//
// It is fs sync in the other direction and shares its contract, its flags and its
// result type — including the rule that nothing is deleted unless --delete was
// asked for, and that --delete then threatens *local* files.
func newFsMirrorCmd() *cobra.Command {
	var o app.FsSyncOptions
	c := &cobra.Command{
		Use:   "mirror <host> <remote-dir> <local-dir>",
		Short: "Download a remote directory",
		Long: `Download the contents of a remote directory into a local one.

The plan is the same shape fs sync reports, so --dry-run shows exactly what would
be written before anything is. --delete prunes local files that are not on the
remote: destructive, refused against a top-level directory or an entire home, and
never implied.`,
		Args: cobra.ExactArgs(3),
		RunE: func(c *cobra.Command, args []string) error {
			o.Host, o.Remote, o.LocalPath = args[0], args[1], args[2]
			audit := startAudit("fs.mirror", o.Host)
			res, aerr := app.NewDefault().FsMirror(c.Context(), o)
			if aerr != nil {
				audit.fail(aerr)
				emitFailure("fs.mirror", o.Host, aerr)
				return nil
			}
			audit.succeed("", "mirror "+o.Remote, nil)
			if jsonFlag {
				_ = output.Success("fs.mirror", o.Host, res).Write(os.Stdout)
			} else {
				renderSyncDirection(res, "downloaded", "would download (dry run, nothing copied)")
			}
			return nil
		},
	}
	c.Flags().BoolVar(&o.Delete, "delete", false,
		"remove local files that are not on the remote (destructive)")
	c.Flags().BoolVar(&o.DryRun, "dry-run", false, "report the plan without copying anything")
	c.Flags().BoolVar(&o.Checksum, "checksum", false, "compare content checksums instead of mtimes")
	c.Flags().StringArrayVar(&o.Excludes, "exclude", nil,
		"rsync exclude pattern (repeatable)")
	c.Flags().DurationVar(&o.Timeout, "timeout", 5*time.Minute, "transfer deadline")
	return c
}

// batchItem is one entry of an fs batch report.
//
// `source` and `destination` are echoed from the manifest so a failed row can be
// matched back to the entry that produced it; the endpoints that actually held
// the bytes are inside `data`, where a directory destination has been resolved.
type batchItem struct {
	Op          string                `json:"op"`
	Source      string                `json:"source"`
	Destination string                `json:"destination"`
	OK          bool                  `json:"ok"`
	Data        *app.FsTransferResult `json:"data,omitempty"`
	Error       *output.ErrorPayload  `json:"error,omitempty"`
}

// batchReport is the data of one fs batch: every entry, in manifest order.
//
// The envelope is `ok: true` when the batch ran to the end, and `failed` counts
// the entries that did not copy. That is deliberate — "the batch completed" and
// "every file arrived" are different claims, and an agent must be able to tell
// them apart without parsing text. The process status is 255 if any entry failed.
type batchReport struct {
	Items     []batchItem `json:"items"`
	Succeeded int         `json:"succeeded"`
	Failed    int         `json:"failed"`
}

// batchEntry is one line of a batch manifest.
type batchEntry struct {
	Op          string `json:"op"`
	Source      string `json:"source"`
	Destination string `json:"destination"`
	Resume      bool   `json:"resume"`
	Checksum    bool   `json:"checksum"`
}

func newFsBatchCmd() *cobra.Command {
	var manifest string
	var timeout time.Duration
	c := &cobra.Command{
		Use:   "batch <host> --manifest <file>",
		Short: "Run an ordered list of put and get operations",
		Long: `Copy a list of file pairs, in order, reporting every entry.

The manifest is a JSON array of
  {"op":"put","source":"./local","destination":"~/remote/path"}
entries, with "get" reversing the direction. An entry may also set "resume" or
"checksum" to take the verified rsync route for that file alone.

Entries run serially, so a shared destination tree stays consistent, and a failed
entry does not stop the ones after it: check data.failed, not just the envelope.`,
		Args: cobra.ExactArgs(1),
		RunE: func(c *cobra.Command, args []string) error {
			host := args[0]
			entries, aerr := readBatchManifest(manifest)
			if aerr != nil {
				emitFailure("fs.batch", host, aerr)
				return nil
			}
			report := runBatch(c, host, entries, timeout)
			if jsonFlag {
				_ = output.Success("fs.batch", host, report).Write(os.Stdout)
			} else {
				for _, item := range report.Items {
					if item.OK {
						fmt.Printf("%s  %s -> %s  (%s)\n", item.Op, item.Source,
							item.Destination, humanBytes(item.Data.Size))
						continue
					}
					fmt.Printf("%s  %s -> %s  %s: %s\n", item.Op, item.Source,
						item.Destination, item.Error.Code, item.Error.Message)
				}
				fmt.Fprintf(os.Stderr, "%d transferred, %d failed\n", report.Succeeded, report.Failed)
			}
			if report.Failed > 0 {
				exitCode = 255
			}
			return nil
		},
	}
	c.Flags().StringVar(&manifest, "manifest", "", "JSON manifest file (required)")
	c.Flags().DurationVar(&timeout, "timeout", 5*time.Minute, "deadline per transfer")
	return c
}

// readBatchManifest loads and validates the manifest before anything is copied, so
// a typo fails the call rather than half-filling a directory tree.
func readBatchManifest(path string) ([]batchEntry, *errs.Error) {
	if path == "" {
		return nil, errs.New(errs.UsageError, "fs batch needs --manifest", false)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, configErr(err)
	}
	var entries []batchEntry
	if err := json.Unmarshal(data, &entries); err != nil {
		return nil, configErr(fmt.Errorf("manifest is not a JSON array of {op,source,destination}: %w", err))
	}
	if len(entries) == 0 {
		return nil, errs.New(errs.ConfigInvalid, "manifest lists nothing to do", false)
	}
	for i, e := range entries {
		if e.Op != "put" && e.Op != "get" {
			return nil, errs.New(errs.ConfigInvalid,
				fmt.Sprintf("manifest entry %d: op must be put or get", i+1), false)
		}
		if e.Source == "" || e.Destination == "" {
			return nil, errs.New(errs.ConfigInvalid,
				fmt.Sprintf("manifest entry %d: source and destination are required", i+1), false)
		}
	}
	return entries, nil
}

// runBatch applies every entry, keeping going past a failure so one missing file
// does not hide the state of the other nineteen.
func runBatch(c *cobra.Command, host string, entries []batchEntry, timeout time.Duration) batchReport {
	a := app.NewDefault()
	report := batchReport{Items: make([]batchItem, 0, len(entries))}
	for _, e := range entries {
		var (
			res    app.FsTransferResult
			aerr   *errs.Error
			audit  = startAudit("fs."+e.Op, host)
			remote = e.Destination
		)
		if e.Op == "get" {
			remote = e.Source
			res, aerr = a.FsGet(c.Context(), app.FsGetOptions{
				Host: host, Remote: e.Source, LocalPath: e.Destination,
				Timeout: timeout, Resume: e.Resume, Checksum: e.Checksum,
			})
		} else {
			res, aerr = a.FsPut(c.Context(), app.FsPutOptions{
				Host: host, LocalPath: e.Source, Remote: e.Destination,
				Timeout: timeout, Resume: e.Resume, Checksum: e.Checksum,
			})
		}
		item := batchItem{Op: e.Op, Source: e.Source, Destination: e.Destination,
			Error: output.ErrorOf(aerr)}
		if aerr != nil {
			audit.fail(aerr)
			report.Failed++
		} else {
			// A copy has no remote exit status of its own, so the entry records the
			// endpoint and leaves exit_code absent (§36).
			audit.succeed("", e.Op+" "+remote, nil)
			report.Succeeded++
			item.Data = &res
		}
		item.OK = aerr == nil
		report.Items = append(report.Items, item)
	}
	return report
}
