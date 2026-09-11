package cli

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"time"

	"github.com/spf13/cobra"
	"github.com/starfield17/rhost/internal/app"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

const maxRemoteHelperBytes = 8 * 1024 * 1024

// remoteFileOps is the set of subcommands that ride the embedded remote helper.
// They are one family, not five commands: same host/path arguments, same JSON
// request, same response envelope, different flags.
var remoteFileOps = map[string]struct {
	args    cobra.PositionalArgs
	use     string
	short   string
	long    string
	timeout time.Duration
}{
	"read": {
		args: cobra.ExactArgs(2), use: "read <host> <path>",
		short: "Print a bounded slice of a remote text file",
		long: "Read a slice of a remote file by line, with the SHA-256 of the whole " +
			"file. Nothing is downloaded: the default is 200 lines and 256 KiB, and " +
			"truncation is reported rather than implied. Feed data.sha256 back to " +
			"`fs write --if-hash` or to a patch.",
		timeout: 60 * time.Second,
	},
	"write": {
		args: cobra.ExactArgs(2), use: "write <host> <path>",
		short: "Create, or hash-guarded replace, a remote text file",
		long: "Write a remote file from a local file or stdin. Creating a new file " +
			"needs no precondition; replacing one requires --if-hash with the " +
			"SHA-256 from `fs read`, so a file that changed since it was read is " +
			"never overwritten. The replacement is atomic within the directory and " +
			"keeps the existing file's permissions.",
		timeout: 60 * time.Second,
	},
	"patch": {
		args: cobra.ExactArgs(2), use: "patch <host> <path>",
		short: "Apply a hash-guarded line-range patch to a remote file",
		long: "Apply edits described in a JSON document: {\"sha256\": \"<from fs read>\", " +
			"\"edits\": [{\"start\": 2, \"end\": 3, \"text\": \"replacement\\n\"}]}. Ranges are " +
			"1-based, inclusive, and refer to the file as it was when the hash was " +
			"taken; overlapping or out-of-range edits are refused with INVALID_PATCH.",
		timeout: 60 * time.Second,
	},
	"grep": {
		args: cobra.RangeArgs(2, 3), use: "grep <host> <pattern> [path]",
		short: "Search remote file contents",
		long: "Search a remote tree with ripgrep. Results are normalised records, " +
			"paginated by offset/limit, and bounded before they leave the host. " +
			"Ignore rules are respected unless --no-ignore is given; no match is an " +
			"empty success, not an error.",
		timeout: 60 * time.Second,
	},
	"glob": {
		args: cobra.RangeArgs(2, 3), use: "glob <host> <pattern> [path]",
		short: "List remote paths matching a glob",
		long: "List files under a remote path whose name matches a glob, sorted by " +
			"path and paginated. Ignore rules are respected unless --no-ignore.",
		timeout: 60 * time.Second,
	},
}

// newRemoteFileCmd builds one helper-backed subcommand.
func newRemoteFileCmd(op string) *cobra.Command {
	spec := remoteFileOps[op]
	var (
		from, hash, patchFile, glob, mode string
		fileMode                          string
		start, lines, limit, offset       int
		maxBytes, contextLines            int
		hidden, noIgnore, ignoreCase      bool
		timeout                           time.Duration
	)
	c := &cobra.Command{
		Use: spec.use, Short: spec.short, Long: spec.long, Args: spec.args,
		RunE: func(c *cobra.Command, args []string) error {
			// Argument validation happens before any SSH connection: an agent
			// that mistypes a flag gets USAGE_ERROR/CONFIG_INVALID at zero cost.
			if e := helperUsageError(op, maxBytes, start, lines, limit, offset, contextLines); e != nil {
				emitFailure("fs."+op, args[0], e)
				return nil
			}
			request := map[string]interface{}{"op": op, "max_bytes": maxBytes}
			if op == "grep" || op == "glob" {
				// The search root defaults to the remote account's home, which is
				// where the helper starts; `.` is resolved there, not here.
				request["pattern"] = args[1]
				request["path"] = "."
				if len(args) == 3 {
					request["path"] = args[2]
				}
				request["limit"], request["offset"] = limit, offset
				request["hidden"], request["no_ignore"] = hidden, noIgnore
				if op == "grep" {
					request["mode"], request["ignore_case"] = mode, ignoreCase
					request["glob"], request["context"] = glob, contextLines
				}
			} else {
				request["path"] = args[1]
				request["if_hash"] = hash
			}
			if op == "read" {
				request["start"], request["lines"] = start, lines
			}
			if op == "write" || op == "patch" {
				data, e := readHelperInput(op, from, patchFile, args[0])
				if e != nil {
					emitFailure("fs."+op, args[0], e)
					return nil
				}
				if op == "write" {
					request["content"] = base64.StdEncoding.EncodeToString(data)
					if fileMode != "" {
						request["file_mode"] = fileMode
					}
				} else if e := loadPatch(request, data, args[0]); e != nil {
					emitFailure("fs.patch", args[0], e)
					return nil
				}
			}

			// Writes are the only remote actions in this family, so they are the
			// only ones the audit trail records: reads and searches are polling,
			// and §36 keeps the trail to actions (docs/ARCHITECTURE.md §36).
			var audit *auditTimer
			if op == "write" || op == "patch" {
				audit = startAudit("fs."+op, args[0])
			}
			res, aerr := app.NewDefault().RemoteFile(c.Context(), args[0], request, timeout)
			if aerr != nil {
				audit.fail(aerr)
				emitFailure("fs."+op, args[0], aerr)
				return nil
			}
			audit.succeed("", op+" "+fmt.Sprint(request["path"]), nil)
			emitHelper(op, args[0], res)
			return nil
		},
	}
	c.Flags().DurationVar(&timeout, "timeout", spec.timeout, "operation deadline")
	c.Flags().IntVar(&maxBytes, "max-bytes", 256*1024, "maximum bytes of results")
	switch op {
	case "read":
		c.Flags().IntVar(&start, "start", 1, "first line (1-based)")
		c.Flags().IntVar(&lines, "lines", 200, "maximum whole lines")
	case "write":
		c.Flags().StringVar(&from, "from", "-", "local file, or - for stdin")
		c.Flags().StringVar(&hash, "if-hash", "", "SHA-256 from fs read, required to replace a file")
		// `--mode` means permissions here and search shape for grep, because these
		// are different commands; the value is checked remotely, as an octal.
		c.Flags().StringVar(&fileMode, "mode", "", "octal permissions, e.g. 0755 (new files default to 0600)")
	case "patch":
		c.Flags().StringVar(&patchFile, "patch", "-", "patch JSON file, or - for stdin")
		c.Flags().StringVar(&hash, "if-hash", "", "SHA-256 from fs read (the patch file usually carries it)")
	case "grep", "glob":
		c.Flags().IntVar(&limit, "limit", 100, "maximum result records")
		c.Flags().IntVar(&offset, "offset", 0, "records to skip (pass the previous next)")
		c.Flags().BoolVar(&hidden, "hidden", false, "include hidden files")
		c.Flags().BoolVar(&noIgnore, "no-ignore", false, "ignore no ignore rules")
		if op == "grep" {
			c.Flags().StringVar(&mode, "mode", "content", "content, files or count")
			c.Flags().BoolVar(&ignoreCase, "ignore-case", false, "case-insensitive match")
			c.Flags().StringVar(&glob, "glob", "", "restrict to paths matching this glob")
			c.Flags().IntVar(&contextLines, "context", 0, "context lines around each match")
		}
	}
	return c
}

// helperUsageError rejects impossible flag values locally. These are the same
// bounds the helper enforces remotely; checking them here costs nothing and
// saves an SSH round trip on a mistake the caller can see immediately.
func helperUsageError(op string, maxBytes, start, lines, limit, offset, context int) *errs.Error {
	if maxBytes <= 0 || maxBytes > maxRemoteHelperBytes {
		return errs.New(errs.ConfigInvalid, "--max-bytes must be between 1 and 8388608", false)
	}
	if op == "read" && (start < 1 || lines < 1) {
		return errs.New(errs.ConfigInvalid, "--start and --lines must be positive", false)
	}
	if op == "grep" || op == "glob" {
		if limit < 1 || offset < 0 {
			return errs.New(errs.ConfigInvalid, "--limit must be positive and --offset non-negative", false)
		}
		if op == "grep" && context < 0 {
			return errs.New(errs.ConfigInvalid, "--context must be non-negative", false)
		}
	}
	return nil
}

// readHelperInput reads the body of a write, or the JSON of a patch. The 8 MiB
// ceiling is the helper's own editing limit, applied here so a hopeless request
// fails before it is encoded and sent.
func readHelperInput(op, from, patchFile, host string) ([]byte, *errs.Error) {
	name := from
	if op == "patch" {
		name = patchFile
	}
	var r io.Reader = os.Stdin
	if name != "-" {
		f, err := os.Open(name)
		if err != nil {
			return nil, configErr(err)
		}
		defer f.Close()
		r = f
	}
	data, err := io.ReadAll(io.LimitReader(r, 8*1024*1024+1))
	if err != nil {
		return nil, configErr(err)
	}
	if len(data) > 8*1024*1024 {
		return nil, errs.New(errs.FileTooLarge, "input exceeds the 8 MiB editing limit", false)
	}
	return data, nil
}

// loadPatch turns the patch document into helper request fields. The hash lives
// in the document so a patch file can be reviewed, kept in a repository, and
// applied without a human copying a 64-digit token onto a command line.
func loadPatch(request map[string]interface{}, data []byte, host string) *errs.Error {
	var p struct {
		Hash  string                   `json:"sha256"`
		Edits []map[string]interface{} `json:"edits"`
	}
	if err := json.Unmarshal(data, &p); err != nil {
		return configErr(fmt.Errorf("patch document is not valid JSON: %w", err))
	}
	if len(p.Edits) == 0 {
		return errs.New(errs.ConfigInvalid, "patch document has no edits", false)
	}
	if p.Hash == "" {
		return errs.New(errs.ConfigInvalid,
			"patch document has no sha256: read the file first and carry its hash", false)
	}
	// An explicit --if-hash overrides the document, which is what lets a reviewed
	// patch file be applied against a freshly read hash.
	if h, ok := request["if_hash"].(string); ok && h != "" {
		p.Hash = h
	}
	request["if_hash"] = p.Hash
	request["edits"] = p.Edits
	return nil
}

// emitHelper renders one helper response. `--json` always carries the whole
// document; the human view is the part of it a person asked for, with the rest
// on stderr, so `rhost fs read host file > copy.txt` stays clean (docs §32).
func emitHelper(op, host string, res map[string]interface{}) {
	operation := "fs." + op
	if jsonFlag {
		_ = output.Success(operation, host, res).Write(os.Stdout)
		return
	}
	switch op {
	case "read":
		content, _ := res["content"].(string)
		fmt.Print(content)
		fmt.Fprintf(os.Stderr, "%s: lines %v-%v of %v (%s)\n", res["path"], res["start"],
			helperLineEnd(res), res["total_lines"], shortHash(res))
		if truncated, _ := res["truncated"].(bool); truncated {
			fmt.Fprintln(os.Stderr, "output was truncated: raise --lines or --max-bytes")
		}
	case "grep", "glob":
		rows, _ := res["results"].([]interface{})
		for _, row := range rows {
			fmt.Println(searchLine(row))
		}
		if len(rows) == 0 {
			fmt.Fprintln(os.Stderr, "no results")
		}
		if truncated, _ := res["truncated"].(bool); truncated {
			fmt.Fprintf(os.Stderr, "truncated: pass --offset %v for the next page\n", res["next"])
		}
	case "write", "patch":
		fmt.Fprintf(os.Stderr, "wrote %s (%v bytes, sha256 %s)\n", res["path"], res["bytes"],
			shortHash(res))
	}
}

// helperLineEnd is the last line the response carried, for the human view only.
func helperLineEnd(res map[string]interface{}) interface{} {
	start, _ := res["start"].(float64)
	n, _ := res["lines"].(float64)
	if n == 0 {
		return res["start"]
	}
	return int(start + n - 1)
}

func shortHash(res map[string]interface{}) interface{} {
	h, _ := res["sha256"].(string)
	if len(h) > 12 {
		return h[:12] + "..."
	}
	if h == "" {
		return "none"
	}
	return h
}

// searchLine renders one normalised search record the way ripgrep would:
// `path:line:text` for a match, `path-count` for a count, a bare path otherwise.
func searchLine(row interface{}) string {
	m, ok := row.(map[string]interface{})
	if !ok {
		return fmt.Sprint(row)
	}
	path, _ := m["path"].(string)
	text, hasText := m["text"].(string)
	switch {
	case hasText && m["context"] == true:
		return fmt.Sprintf("%s-%s", path, text)
	case hasText:
		return fmt.Sprintf("%s:%v:%s", path, m["line"], text)
	case m["count"] != nil:
		return fmt.Sprintf("%s:%v", path, m["count"])
	default:
		return path
	}
}
