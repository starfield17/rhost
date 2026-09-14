package fileops

import (
	"path"
	"path/filepath"
	"strings"
)

// ChangeMarker prefixes every itemized line rsync emits. It exists because the
// same stdout also carries warnings, a remote motd, and (in openrsync) the
// deletion notices that ignore --out-format: a parser has to tell those apart
// from plan lines, not guess from shape.
const ChangeMarker = "RHOSTSYNC|"

// RejectSyncTarget decides whether a directory-sync destination is safe enough to
// write to — and, when --delete is set, safe enough to *prune* (§28: "sync is the
// operation most likely to cause accidental data loss").
//
// The rules are deliberately few and mechanical:
//
//  1. a destination is never empty, the root, a glob, or carrying control
//     characters (shared with single-file transfers);
//  2. with --delete, an absolute destination must have at least two path
//     components. `/srv` may be exactly where a database lives; `/srv/app/deploy`
//     is a directory some name was chosen for. Refusing the one-component case is
//     the difference between a mistake and a disaster — and it is explainable:
//     rhost says so, and syncing into a subdirectory is the way round it;
//  3. with --delete, `~` or `~user` alone (someone's entire home) is refused for
//     the same reason;
//  4. `.` and `..` are refused always: what they name depends on the remote
//     tool's working directory, which rhost does not control.
func RejectSyncTarget(destination string, delete bool) error {
	if err := ValidateRemotePath(destination); err != nil {
		return err
	}
	destination = path.Clean(destination)
	if err := checkDestination(destination); err != nil {
		return err
	}
	switch destination {
	case ".", "./", "..", "../":
		return reject("sync destination %q depends on the remote working directory; use a path under the home directory or an absolute path", destination)
	}
	if !delete {
		return nil
	}
	if destination == "~" || destination == "~/" {
		return reject("--delete refuses an entire home directory (%q); choose a subdirectory", destination)
	}
	if strings.HasPrefix(destination, "~") && !strings.Contains(strings.TrimPrefix(destination, "~"), "/") {
		return reject("--delete refuses an entire home directory (%q); choose a subdirectory", destination)
	}
	if filepath.IsAbs(destination) && len(strings.Split(strings.Trim(destination, "/"), "/")) == 1 {
		return reject("--delete refuses a top-level directory (%q); sync into a subdirectory instead", destination)
	}
	return nil
}

// SyncOptions describes one directory sync.
//
// The source's *contents* are transferred: the trailing slash is added here
// rather than left to the caller, because "src" and "src/" mean different things
// to rsync and rhost must not depend on which one a human typed.
type SyncOptions struct {
	Source      string
	Destination string
	Delete      bool
	DryRun      bool
	Excludes    []string
}

// SyncArgs builds the rsync argv for one local→remote sync. sshOpts is the same
// `-o Key=Value` list rhost passes to ssh, so the transfer rides the existing
// ControlMaster instead of authenticating again (§8).
//
// rsync splits its own -e string on whitespace, so an option containing a space
// cannot travel that way: rhost then syncs over a fresh connection rather than
// mangling the argument, and reports which happened through multiplexed.
func SyncArgs(host string, o SyncOptions, sshOpts []string) (args []string, remote string, multiplexed bool, err error) {
	if err := RejectSyncTarget(o.Destination, o.Delete); err != nil {
		return nil, "", false, err
	}
	src, err := LocalArg(o.Source)
	if err != nil {
		return nil, "", false, err
	}

	args = []string{"--recursive"}
	// --times preserves mtimes, so a later sync is incremental rather than "the
	// whole tree changed". --no-owner/--no-group are explicit rather than left to
	// a default: preserving ownership would need root on the remote, and a sync
	// must not quietly fail at it. Symlinks are skipped rather than followed,
	// which is what both GNU rsync and the openrsync shipped by macOS do without
	// --links (§28: do not follow unexpected symlinks across boundaries).
	args = append(args, "--times", "--no-owner", "--no-group", "--no-perms")
	if o.DryRun {
		args = append(args, "--dry-run")
	}
	if o.Delete {
		args = append(args, "--delete")
	}
	for _, pat := range o.Excludes {
		if strings.ContainsAny(pat, "\n\x00") {
			return nil, "", false, reject("exclude pattern %q contains a newline or NUL", pat)
		}
		args = append(args, "--exclude", pat)
	}
	args = append(args, "--itemize-changes", "--out-format="+ChangeMarker+"%i|%n")

	shell, usable := remoteShell(sshOpts)
	args = append(args, "-e", shell)

	srcArg := strings.TrimRight(src, "/") + "/"
	args = append(args, srcArg, RemoteSpec(host, o.Destination))
	return args, RemoteSpec(host, o.Destination), usable, nil
}

// remoteShell renders rsync's -e value and reports whether it still carries rhost's
// options (false means the transfer cannot reuse the multiplexed connection).
//
// rsync splits its own -e string with shell-like rules, so an option containing
// whitespace is single-quoted instead of dropped: without the quotes the option
// would reach ssh as two arguments, and the old fallback turned that into a whole
// non-multiplexed transfer. Quoting was verified against GNU rsync and against
// the openrsync macOS ships; the shell-style `'\”` escape was *not* (openrsync
// rejects it as an unterminated quote), so an option containing a single quote
// still falls back rather than being mangled into something nearer to a bug.
func remoteShell(sshOpts []string) (string, bool) {
	quoted := make([]string, 0, len(sshOpts)+1)
	for _, o := range sshOpts {
		if strings.ContainsAny(o, "\n\x00") {
			return "ssh", false
		}
		q, ok := quoteRSH(o)
		if !ok {
			return "ssh", false
		}
		quoted = append(quoted, q)
	}
	if len(quoted) == 0 {
		return "ssh", false
	}
	return "ssh " + strings.Join(quoted, " "), true
}

// quoteRSH single-quotes an option that contains whitespace, which rsync's own
// splitter removes again before ssh sees the argument. An option that cannot be
// represented that way reports false, and the caller syncs over a fresh
// connection instead.
func quoteRSH(o string) (string, bool) {
	if !strings.ContainsAny(o, " \t") {
		return o, true
	}
	if strings.Contains(o, "'") {
		return "", false
	}
	return "'" + o + "'", true
}

// Action is what a change line says is about to happen. Coarse on purpose: the
// itemize string is kept beside it so nothing is hidden, while an agent can still
// branch on a small stable set.
type Action string

const (
	ActionCreate    Action = "create"
	ActionUpdate    Action = "update"
	ActionDelete    Action = "delete"
	ActionDirectory Action = "directory"
	ActionSkip      Action = "skip"
	ActionOther     Action = "other"
)

// Change is one line of rsync's itemized plan.
type Change struct {
	Action  Action `json:"action"`
	Path    string `json:"path"`
	Itemize string `json:"itemize"`
}

// ParseChanges turns rsync's stdout into structured changes. Lines without the
// marker (warnings, openrsync's delete notices) are returned separately so a
// human can still see what the tool said; they are never dropped.
func ParseChanges(out string) (changes []Change, other []string) {
	for _, line := range strings.Split(strings.ReplaceAll(out, "\r\n", "\n"), "\n") {
		switch {
		case line == "":
			continue
		case strings.HasPrefix(line, ChangeMarker):
			// The itemize column is fixed-width text with no "|" in it, so the
			// first separator is the only one to split on: everything after it is
			// the filename, "|" characters and all.
			itemize, name, found := strings.Cut(strings.TrimPrefix(line, ChangeMarker), "|")
			if !found {
				other = append(other, line)
				continue
			}
			changes = append(changes, Change{Action: classify(itemize), Path: name, Itemize: itemize})
		case strings.HasPrefix(line, "*deleting"):
			// openrsync prints deletions in its own format; GNU rsync puts them
			// through --out-format, so this branch is the other half of one rule.
			name := strings.TrimSpace(strings.TrimPrefix(line, "*deleting"))
			changes = append(changes, Change{Action: ActionDelete, Path: name, Itemize: "*deleting"})
		default:
			other = append(other, line)
		}
	}
	return changes, other
}

// classify maps an rsync itemize string to an Action.
//
// Only the leading characters are load-bearing: `>f` is a file being sent to the
// destination, `cd`/`.d` a directory, and a run of `+` in the transfer column
// means "created" while letters mean "something about it changed".
func classify(itemize string) Action {
	switch {
	case strings.HasPrefix(itemize, "*deleting"):
		return ActionDelete
	// The second character names the file type, and that is what decides whether
	// the entry is a file at all.
	case len(itemize) > 1 && itemize[1] == 'd':
		return ActionDirectory
	case len(itemize) > 1 && strings.IndexByte("Lcsbp-", itemize[1]) >= 0:
		// A symlink, socket, device or special file is neither copied nor
		// followed (§28: nothing follows a symlink across the boundary silently),
		// so it is reported as skipped rather than looking like a missing file.
		return ActionSkip
	case len(itemize) > 2 && strings.HasPrefix(itemize, ">f"):
		if strings.Trim(itemize[2:], "+") == "" {
			return ActionCreate
		}
		return ActionUpdate
	case len(itemize) > 2 && strings.HasPrefix(itemize, "cf"):
		return ActionCreate
	default:
		return ActionOther
	}
}
