package app

import (
	"context"
	"errors"
	"os"
	"path"
	"path/filepath"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/starfield17/rhost/internal/config"
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/fileops"
	"github.com/starfield17/rhost/internal/shell"
)

// FsPutOptions is one local → remote file copy.
//
// The local side is LocalPath rather than Local on purpose: a field selector
// ending in "Local" reads as an mDNS host suffix to scripts/check-portability.sh
// (AGENTS.md §1), and that script is deliberately kept strict rather than taught
// to special-case Go field names.
type FsPutOptions struct {
	Host      string
	LocalPath string
	Remote    string
	Timeout   time.Duration
	// Resume and Checksum opt into the verified rsync route: partial-file
	// resumption, and an end-to-end SHA-256 comparison after the copy. Both need
	// rsync at both ends, and the checksum needs a remote `sha256sum`.
	Resume   bool
	Checksum bool
	Parents  bool
}

// FsGetOptions is one remote → local file copy.
type FsGetOptions struct {
	Host string
	// Remote names the file on the far side. LocalPath is `destination` as the
	// user typed it: when it names an existing directory, the result reports the
	// file inside it.
	LocalPath string
	Remote    string
	Timeout   time.Duration
	// Resume and Checksum behave exactly as in FsPutOptions, in the other
	// direction.
	Resume   bool
	Checksum bool
}

// FsSyncOptions is one directory sync, in either direction: FsSync pushes the
// local tree, FsMirror pulls a remote one.
type FsSyncOptions struct {
	Host      string
	LocalPath string
	Remote    string
	Delete    bool
	DryRun    bool
	Excludes  []string
	Timeout   time.Duration
	// Checksum compares file content instead of size and mtime. It changes what
	// rsync considers a difference; it is not the end-to-end verification that
	// `fs put --checksum` performs.
	Checksum bool
}

// FsTransferResult is the outcome of a single-file copy. `source` and
// `destination` are the effective endpoints (§28: return them, so what happened
// is never a matter of interpretation), and `backend` says which tool did it.
type FsTransferResult struct {
	Source      string `json:"source"`
	Destination string `json:"destination"`
	Backend     string `json:"backend"`
	Size        int64  `json:"size"`
	Multiplexed bool   `json:"multiplexed"`
	DurationMS  int64  `json:"duration_ms"`
	// ResumeEnabled describes the mode that was asked for, not a claim that bytes
	// were actually reused — a first copy resumes from nothing. ChecksumVerified
	// is the stronger statement: both ends hashed the same content.
	ChecksumVerified bool `json:"checksum_verified"`
	ResumeEnabled    bool `json:"resume_enabled"`
}

// FsSyncResult is the outcome of one sync — including a dry run, where `changes`
// is the plan and nothing moved. `deletes` counts the pruning actions so a caller
// can see the destructive part of a plan without parsing the list.
type FsSyncResult struct {
	Source      string           `json:"source"`
	Destination string           `json:"destination"`
	Backend     string           `json:"backend"`
	DryRun      bool             `json:"dry_run"`
	Delete      bool             `json:"delete"`
	Multiplexed bool             `json:"multiplexed"`
	Changes     []fileops.Change `json:"changes"`
	Files       int              `json:"files"`
	Directories int              `json:"directories"`
	Deletes     int              `json:"deletes"`
	Notes       []string         `json:"notes,omitempty"`
	DurationMS  int64            `json:"duration_ms"`
}

// transferTimeout is the budget for a file operation. It is longer than exec's
// default on purpose: a copy's duration belongs to the size of the data, not to
// the command, and a silent 60s ceiling would make `fs put` of a real artifact
// unusable.
const transferTimeout = 5 * time.Minute

func (o *FsPutOptions) timeout() time.Duration {
	if o.Timeout <= 0 {
		return transferTimeout
	}
	return o.Timeout
}

func (o *FsGetOptions) timeout() time.Duration {
	if o.Timeout <= 0 {
		return transferTimeout
	}
	return o.Timeout
}

func (o *FsSyncOptions) timeout() time.Duration {
	if o.Timeout <= 0 {
		return transferTimeout
	}
	return o.Timeout
}

// scpReusesTransport reports whether scp was given rhost's own ControlPath
// options: it always is today, and the field exists so the JSON never claims
// multiplexing it did not get.
func scpReusesTransport(sshOpts []string) bool {
	for _, o := range sshOpts {
		if strings.HasPrefix(o, "ControlPath=") {
			return true
		}
	}
	return false
}

// mapTransferErr turns a tool-level failure into the taxonomy. The tool's own
// first complaint is kept in the message because "scp exited 1" tells an agent
// nothing it can act on; the code stays stable either way.
func mapTransferErr(res fileops.Result, stderrPrefix string) *errs.Error {
	if res.TimedOut {
		return errs.New(errs.RemoteCommandTimeout, "transfer exceeded its timeout", true)
	}
	// The first line only: scp and rsync put their one real complaint first, and a
	// transfer tool can also dribble a carriage-return progress bar into stderr.
	msg := clip(firstLine(string(res.Stderr)), 300)
	if msg == "" {
		msg = clip(firstLine(string(res.Stdout)), 300)
	}
	if msg == "" {
		msg = "no diagnostic"
	}
	return errs.New(errs.TransferFailed, stderrPrefix+": "+msg, true)
}

// clip bounds a message taken from a subprocess, in bytes, at a rune boundary.
func clip(s string, max int) string {
	if len(s) <= max {
		return s
	}
	cut := s[:max]
	for !utf8.ValidString(cut) && len(cut) > 0 {
		cut = cut[:len(cut)-1]
	}
	return cut + "..."
}

// FsPut copies one local file to a remote path.
func (a *App) FsPut(ctx context.Context, opts FsPutOptions) (FsTransferResult, *errs.Error) {
	local, err := fileops.LocalArg(opts.LocalPath)
	if err != nil {
		return FsTransferResult{}, transferValidation(err)
	}
	dst := fileops.RemoteSpec(opts.Host, opts.Remote)
	if err := fileops.ValidateTransferPaths(local, dst); err != nil {
		return FsTransferResult{}, transferValidation(err)
	}
	info, aerr := a.statLocalFile(local, "put")
	if aerr != nil {
		return FsTransferResult{}, aerr
	}
	if opts.Parents {
		if e := a.ensureRemoteParent(ctx, opts.Host, opts.Remote, opts.timeout()); e != nil {
			return FsTransferResult{}, e
		}
	}
	if opts.Resume || opts.Checksum {
		return a.verifiedTransfer(ctx, opts.Host, opts.LocalPath, opts.Remote, false, opts.Resume, opts.Checksum, opts.timeout())
	}
	effectiveRemote, aerr := a.remoteFileOf(ctx, opts.Host, opts.Remote, filepath.Base(local), opts.timeout())
	if aerr != nil {
		return FsTransferResult{}, aerr
	}
	effectiveDst := fileops.RemoteSpec(opts.Host, effectiveRemote)

	sshOpts := a.SSH.SSHOptions()
	argv, err := fileops.ScpArgs(local, dst, sshOpts)
	if err != nil {
		return FsTransferResult{}, transferValidation(err)
	}
	res, runErr := a.runTool(ctx, a.Transfers.ScpBin, argv, opts.timeout())
	if runErr != nil {
		return FsTransferResult{}, missingTool(errs.TransferFailed, "scp", runErr)
	}
	if res.ExitCode != 0 {
		return FsTransferResult{}, mapTransferErr(res, "scp could not copy the file")
	}
	mux := scpReusesTransport(sshOpts) && a.SSH.MasterAlive(ctx, opts.Host)
	return FsTransferResult{
		Source: local, Destination: effectiveDst, Backend: fileops.BackendScp,
		Size: info.Size(), Multiplexed: mux, DurationMS: res.Duration.Milliseconds(),
	}, nil
}

func (a *App) ensureRemoteParent(ctx context.Context, host, remote string, timeout time.Duration) *errs.Error {
	command := remoteParentCommand(remote)
	if command == "" {
		return nil
	}
	res, e := a.Execute(ctx, ExecOptions{Host: host, Command: command, Timeout: timeout})
	if e != nil {
		return e
	}
	if res.ExitCode != 0 {
		return errs.New(errs.TransferFailed, "could not create remote parent directory: "+firstLine(res.Stderr), false)
	}
	return nil
}

func remoteParentCommand(remote string) string {
	trimmed := strings.TrimRight(remote, "/")
	parent := path.Dir(trimmed)
	if strings.HasSuffix(remote, "/") {
		parent = trimmed
	}
	if parent == "." || parent == "" {
		return ""
	}
	return "mkdir -p -- " + shell.PathQuote(parent)
}

// FsGet copies one remote file to a local path.
func (a *App) FsGet(ctx context.Context, opts FsGetOptions) (FsTransferResult, *errs.Error) {
	if opts.Resume || opts.Checksum {
		return a.verifiedTransfer(ctx, opts.Host, opts.LocalPath, opts.Remote, true, opts.Resume, opts.Checksum, opts.timeout())
	}
	src := fileops.RemoteSpec(opts.Host, opts.Remote)
	local, err := fileops.LocalArg(opts.LocalPath)
	if err != nil {
		return FsTransferResult{}, transferValidation(err)
	}
	if err := fileops.ValidateTransferPaths(src, local); err != nil {
		return FsTransferResult{}, transferValidation(err)
	}

	sshOpts := a.SSH.SSHOptions()
	argv, err := fileops.ScpArgs(src, local, sshOpts)
	if err != nil {
		return FsTransferResult{}, transferValidation(err)
	}
	res, runErr := a.runTool(ctx, a.Transfers.ScpBin, argv, opts.timeout())
	if runErr != nil {
		return FsTransferResult{}, missingTool(errs.TransferFailed, "scp", runErr)
	}
	if res.ExitCode != 0 {
		return FsTransferResult{}, mapTransferErr(res, "scp could not fetch the file")
	}

	// scp writes into an existing directory rather than replacing it, so the
	// effective destination is the file it created. Reporting the directory would
	// leave an agent to guess the name.
	out := local
	if st, statErr := os.Stat(local); statErr == nil && st.IsDir() {
		out = filepath.Join(local, pathBase(opts.Remote))
	}
	size := int64(0)
	if st, statErr := os.Stat(out); statErr == nil && !st.IsDir() {
		size = st.Size()
	}
	mux := scpReusesTransport(sshOpts) && a.SSH.MasterAlive(ctx, opts.Host)
	return FsTransferResult{
		Source: src, Destination: out, Backend: fileops.BackendScp,
		Size: size, Multiplexed: mux, DurationMS: res.Duration.Milliseconds(),
	}, nil
}

// FsSync brings a remote directory in line with a local one. Nothing is deleted
// unless --delete was asked for, and --dry-run reports the plan without running
// any of it (docs/architecture/files-and-json.md).
func (a *App) FsSync(ctx context.Context, opts FsSyncOptions) (FsSyncResult, *errs.Error) {
	local, err := fileops.LocalArg(opts.LocalPath)
	if err != nil {
		return FsSyncResult{}, transferValidation(err)
	}

	// The destination rule is judged before anything else, including the local
	// directory: when both ends of a `--delete` sync are wrong, the answer an
	// operator needs first is the refusal to prune a top-level path.
	o := fileops.SyncOptions{
		Source: local, Destination: opts.Remote,
		Delete: opts.Delete, DryRun: opts.DryRun, Excludes: opts.Excludes,
	}
	argv, remote, mux, err := fileops.SyncArgs(opts.Host, o, a.SSH.SSHOptions())
	if err != nil {
		return FsSyncResult{}, transferValidation(err)
	}

	if _, aerr := a.statLocalDir(local); aerr != nil {
		return FsSyncResult{}, aerr
	}

	// rsync must exist at both ends. Saying "the remote has no rsync" is a
	// capability answer that doctor already gives; discovering it through rsync's
	// own exit status would report a tool failure instead.
	if aerr := a.remoteHasRsync(ctx, opts.Host); aerr != nil {
		return FsSyncResult{}, aerr
	}
	if opts.Delete {
		// A local check cannot see that the remote `/srv/app` is a symlink to `/`,
		// so a destructive sync also asks the remote what the destination actually
		// names, and syncs to *that* (docs/architecture/files-and-json.md).
		resolved, e := a.RemoteFile(ctx, opts.Host, map[string]interface{}{
			"op": "resolve", "path": opts.Remote, "delete": true,
		}, opts.timeout())
		if e != nil {
			return FsSyncResult{}, e
		}
		actual, ok := resolved["path"].(string)
		if !ok {
			return FsSyncResult{}, errs.New(errs.Internal, "remote destination probe returned no path", false)
		}
		o.Destination = actual
		argv, remote, mux, err = fileops.SyncArgs(opts.Host, o, a.SSH.SSHOptions())
		if err != nil {
			return FsSyncResult{}, transferValidation(err)
		}
	}
	if opts.Checksum {
		argv = append([]string{"--checksum"}, argv...)
	}

	res, runErr := a.runTool(ctx, a.Transfers.RsyncBin, argv, opts.timeout())
	if runErr != nil {
		return FsSyncResult{}, missingTool(errs.TransferFailed, "rsync", runErr)
	}
	if res.ExitCode != 0 {
		return FsSyncResult{}, mapTransferErr(res, "rsync could not sync the directory")
	}
	changes, notes := fileops.ParseChanges(string(res.Stdout))
	mux = mux && a.SSH.MasterAlive(ctx, opts.Host)
	out := FsSyncResult{
		Source: strings.TrimRight(local, "/") + "/", Destination: remote,
		Backend: fileops.BackendRsync, DryRun: opts.DryRun, Delete: opts.Delete,
		Multiplexed: mux, Changes: changes, Notes: notes,
		DurationMS: res.Duration.Milliseconds(),
	}
	for _, ch := range changes {
		switch ch.Action {
		case fileops.ActionDelete:
			out.Deletes++
		case fileops.ActionCreate, fileops.ActionUpdate:
			out.Files++
		case fileops.ActionDirectory:
			out.Directories++
		}
	}
	return out, nil
}

// statLocalFile checks the source of a put is one regular file. A directory is a
// clear "use the other command" answer, not something to guess at.
func (a *App) statLocalFile(path, verb string) (os.FileInfo, *errs.Error) {
	st, err := os.Stat(path)
	if err != nil {
		return nil, errs.Wrap(errs.ConfigInvalid, "cannot read "+path+": "+err.Error(), false, err)
	}
	if st.IsDir() {
		return nil, errs.New(errs.ConfigInvalid,
			"fs "+verb+" copies one file; use fs sync for a directory", false)
	}
	if !st.Mode().IsRegular() {
		return nil, errs.New(errs.ConfigInvalid,
			"fs "+verb+" needs a regular file, not "+path, false)
	}
	return st, nil
}

func (a *App) statLocalDir(path string) (os.FileInfo, *errs.Error) {
	st, err := os.Stat(path)
	if err != nil {
		return nil, errs.Wrap(errs.ConfigInvalid, "cannot read "+path+": "+err.Error(), false, err)
	}
	if !st.IsDir() {
		return nil, errs.New(errs.ConfigInvalid,
			"fs sync takes a local directory; use fs put for one file", false)
	}
	return st, nil
}

// remoteHasRsync probes the one dependency sync cannot work without. It runs
// through the same exec path as everything else, so OpenSSH still resolves the
// host and owns the connection.
func (a *App) remoteHasRsync(ctx context.Context, host string) *errs.Error {
	// A bare `command -v`, not the exec wrapper: the question is yes/no, and the
	// answer must not depend on bash, markers, or the state directory.
	res, err := a.SSH.Run(ctx, host, "sh -c 'command -v rsync >/dev/null 2>&1'", 30*time.Second)
	switch {
	case err != nil:
		return errs.Wrap(errs.SSHUnreachable, "could not ask the remote host about rsync: "+err.Error(), true, err)
	case res.TimedOut:
		return errs.New(errs.RemoteCommandTimeout, "the rsync probe exceeded its timeout", true)
	case res.ExitCode == 0:
		return nil
	}
	if res.ExitCode == 255 {
		if e := classifySSH(string(res.Stderr)); e != nil {
			return e
		}
		return errs.New(errs.SSHUnreachable, "could not probe rsync: "+firstLine(string(res.Stderr)), true)
	}
	return errs.New(errs.RemoteDependencyMissing,
		"the remote host has no rsync, which fs sync needs; fs put and fs get work without it", false)
}

// runTool starts one local transfer tool. The ControlPath directory is created
// first because a put or get may be the very first thing rhost does on this
// machine, and scp cannot open a master through a directory that is missing.
func (a *App) runTool(ctx context.Context, bin string, args []string, timeout time.Duration) (fileops.Result, error) {
	if err := config.EnsureControlDir(); err != nil {
		return fileops.Result{}, err
	}
	if bin == a.Transfers.RsyncBin {
		var err error
		args, err = a.Transfers.SafeRsyncPaths(ctx, args)
		if err != nil {
			return fileops.Result{}, err
		}
	}
	return a.Transfers.Run(ctx, bin, args, timeout)
}

// transferValidation maps fileops' refusals onto the taxonomy. Every one of them
// is a decision made before anything runs, so an agent can fix the invocation
// instead of retrying. A failed transfer is reported retryable: the common causes
// are a dropped connection or a full disk elsewhere, and rhost does not sort
// those by matching the tool's English text (§33).
func transferValidation(err error) *errs.Error {
	if rejected, ok := err.(fileops.ErrRejected); ok {
		return errs.New(errs.SyncRejected, rejected.Error(), false)
	}
	return errs.Wrap(errs.TransferFailed, err.Error(), false, err)
}

// missingTool distinguishes "the tool refused" from "the tool is not installed
// here". The second one is not the remote's fault and not retryable.
func missingTool(code errs.Code, name string, cause error) *errs.Error {
	if errors.Is(cause, config.ErrUnsafeLocalState) {
		return errs.Wrap(errs.ConfigInvalid, cause.Error(), false, cause)
	}
	msg := "cannot run " + name + " on this machine: " + cause.Error()
	if os.IsNotExist(cause) || strings.Contains(cause.Error(), "executable file not found") {
		return errs.Wrap(errs.TransferFailed, name+" is not installed locally", false, cause)
	}
	return errs.Wrap(code, msg, false, cause)
}

// pathBase is filepath.Base for a remote path, which may use forward slashes on
// any platform (`C:/x/y` aside, rhost treats remote paths as POSIX).
func pathBase(remote string) string {
	if _, p, ok := fileops.SplitRemoteSpec(remote); ok {
		remote = p
	}
	remote = strings.TrimRight(remote, "/")
	if i := strings.LastIndexByte(remote, '/'); i >= 0 {
		remote = remote[i+1:]
	}
	if remote == "" {
		return "rhost-download"
	}
	return remote
}
