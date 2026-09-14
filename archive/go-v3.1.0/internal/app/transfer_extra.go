package app

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/fileops"
	"github.com/starfield17/rhost/internal/shell"
)

// verifiedTransfer is the `--resume` / `--checksum` path of fs put and fs get.
//
// Plain fs put/fs get use scp, which needs nothing on the far side but a shell.
// A verified copy is a different promise, so it is a different route: rsync with
// content comparison, a resumable partial file, and a SHA-256 computed at both
// ends after the copy. That costs three remote round trips and requires rsync on
// both sides plus a remote `sha256sum`, which is exactly why it is opt-in
// (docs/architecture/files-and-json.md: report which backend did the work).
//
// The hash is the last word. A transfer that "succeeded" but whose bytes differ
// is a failure, because the only reason to ask for verification is to be told
// when the two ends disagree.
func (a *App) verifiedTransfer(ctx context.Context, host, local, remote string, download, resume, checksum bool, timeout time.Duration) (FsTransferResult, *errs.Error) {
	local, err := fileops.LocalArg(local)
	if err != nil {
		return FsTransferResult{}, transferValidation(err)
	}
	source, destination := local, fileops.RemoteSpec(host, remote)
	if download {
		source, destination = destination, local
	} else if _, e := a.statLocalFile(local, "put"); e != nil {
		return FsTransferResult{}, e
	}
	if err = fileops.ValidateTransferPaths(source, destination); err != nil {
		return FsTransferResult{}, transferValidation(err)
	}
	if e := a.remoteHasRsync(ctx, host); e != nil {
		return FsTransferResult{}, e
	}

	ssh := []string{"ssh"}
	for _, o := range a.SSH.SSHOptions() {
		ssh = append(ssh, shell.Quote(o))
	}
	args := []string{"--times", "--checksum", "-e", strings.Join(ssh, " ")}
	if resume {
		// A dedicated partial dir, so an interrupted rhost transfer never mixes
		// with a `.pid`-style partial left by some other rsync invocation, and a
		// later run can pick the bytes up instead of starting over.
		args = append(args, "--partial-dir=.rhost-partial")
	}
	args = append(args, "--", source, destination)

	res, err := a.runTool(ctx, a.Transfers.RsyncBin, args, timeout)
	if err != nil {
		return FsTransferResult{}, missingTool(errs.TransferFailed, "rsync", err)
	}
	if res.ExitCode != 0 {
		return FsTransferResult{}, mapTransferErr(res, "rsync transfer failed")
	}

	// Both tools write to the *named* destination, which for a directory means
	// the file inside it. The reported endpoints are the ones that actually hold
	// the bytes, not the ones the operator typed (§28: return what happened).
	remoteFile := remote
	if download {
		if st, e := os.Stat(local); e == nil && st.IsDir() {
			local = filepath.Join(local, pathBase(remote))
			destination = local
		}
	} else if target, e := a.remoteFileOf(ctx, host, remote, filepath.Base(local), timeout); e != nil {
		return FsTransferResult{}, e
	} else {
		remoteFile = target
		destination = fileops.RemoteSpec(host, remoteFile)
	}

	want, size, e := a.localSha256(local)
	if e != nil {
		return FsTransferResult{}, e
	}
	got, e := a.remoteSha256(ctx, host, remoteFile, timeout)
	if e != nil {
		return FsTransferResult{}, e
	}
	if got != want {
		return FsTransferResult{}, errs.New(errs.TransferFailed,
			"end-to-end SHA-256 verification failed for "+remoteFile, true)
	}
	_ = checksum // every verified route compares content; there is no weaker one
	return FsTransferResult{
		Source: source, Destination: destination,
		Backend: fileops.BackendRsync, Multiplexed: a.SSH.MasterAlive(ctx, host),
		Size: size, DurationMS: res.Duration.Milliseconds(),
		ChecksumVerified: true, ResumeEnabled: resume,
	}, nil
}

// remoteFileOf reports the file a copy landed on: the named path, or the same
// base name inside it when the name is an existing directory.
func (a *App) remoteFileOf(ctx context.Context, host, remote, base string, timeout time.Duration) (string, *errs.Error) {
	probe := "p=" + shell.PathQuote(remote) + "; " +
		"if [ -d \"$p\" ]; then printf '%s/%s' \"$p\" " + shell.Quote(base) +
		"; else printf '%s' \"$p\"; fi"
	res, err := a.SSH.Run(ctx, host, "sh -c "+shell.Quote(probe), timeout)
	if err != nil {
		return "", errs.Wrap(errs.SSHUnreachable,
			"cannot resolve the remote destination: "+err.Error(), true, err)
	}
	if res.TimedOut {
		return "", errs.New(errs.RemoteCommandTimeout,
			"resolving the remote destination exceeded its timeout", true)
	}
	if res.ExitCode == 255 {
		if e := classifySSH(string(res.Stderr), res.ExitCode); e != nil {
			return "", e
		}
	}
	out := string(res.Stdout)
	if res.ExitCode != 0 || out == "" {
		return "", errs.New(errs.TransferFailed, "cannot resolve the remote destination of the copy", false)
	}
	return out, nil
}

// localSha256 is the reference the remote digest is compared against.
func (a *App) localSha256(path string) (string, int64, *errs.Error) {
	f, err := os.Open(path)
	if err != nil {
		return "", 0, errs.Wrap(errs.TransferFailed, err.Error(), false, err)
	}
	defer f.Close()
	h := sha256.New()
	size, err := io.Copy(h, f)
	if err != nil {
		return "", 0, errs.Wrap(errs.TransferFailed, err.Error(), true, err)
	}
	return hex.EncodeToString(h.Sum(nil)), size, nil
}

// remoteSha256 asks the remote for a digest. 127 is the *tool* being absent,
// which is a capability answer and not a checksum mismatch — telling those apart
// is what lets an agent fix the right problem.
func (a *App) remoteSha256(ctx context.Context, host, remote string, timeout time.Duration) (string, *errs.Error) {
	res, e := a.Execute(ctx, ExecOptions{
		Host:    host,
		Command: "sha256sum -- " + shell.PathQuote(remote),
		Timeout: timeout,
	})
	if e != nil {
		return "", e
	}
	if res.ExitCode == 127 {
		return "", errs.New(errs.RemoteDependencyMissing,
			"verified transfer needs sha256sum on the remote host", false)
	}
	fields := strings.Fields(res.Stdout)
	if res.ExitCode != 0 || len(fields) == 0 {
		return "", errs.New(errs.TransferFailed, "cannot read the remote checksum of "+remote, true)
	}
	return fields[0], nil
}

// FsMirror downloads a remote directory.
//
// It is `fs sync` turned around, and it keeps the same contract: contents not
// directories, nothing deleted unless `--delete` was asked for, `--dry-run`
// reports a plan, and a destructive mirror is refused when the local end is a
// top-level directory or an entire home. The difference is the direction of the
// risk: here it is *this* machine's files that could be pruned, so the guard is
// applied to the resolved local path, symlinks included.
func (a *App) FsMirror(ctx context.Context, opts FsSyncOptions) (FsSyncResult, *errs.Error) {
	local, err := filepath.Abs(opts.LocalPath)
	if err != nil {
		return FsSyncResult{}, transferValidation(err)
	}
	if err = fileops.RejectSyncTarget(local, opts.Delete); err != nil {
		return FsSyncResult{}, transferValidation(err)
	}
	if opts.Delete {
		// The guard is applied to what the path *is*, after symlinks, for the same
		// reason as on the remote side. A destination that does not exist yet has
		// nothing to prune, so there is nothing to resolve.
		if _, err := os.Lstat(local); err == nil {
			actual, err := filepath.EvalSymlinks(local)
			if err != nil {
				return FsSyncResult{}, transferValidation(err)
			}
			if home, _ := os.UserHomeDir(); actual == home {
				return FsSyncResult{}, errs.New(errs.SyncRejected,
					"--delete refuses an entire home directory; mirror into a subdirectory", false)
			}
			if err = fileops.RejectSyncTarget(actual, true); err != nil {
				return FsSyncResult{}, transferValidation(err)
			}
			local = actual
		} else if !os.IsNotExist(err) {
			return FsSyncResult{}, transferValidation(err)
		}
	}
	if strings.TrimSpace(opts.Remote) == "" {
		return FsSyncResult{}, errs.New(errs.ConfigInvalid, "a mirror needs a remote source directory", false)
	}
	if e := a.remoteHasRsync(ctx, opts.Host); e != nil {
		return FsSyncResult{}, e
	}

	// The remote end is the source here, so the destination guard is spent on the
	// local directory and the placeholder keeps rsync's argv shape identical.
	args, _, mux, err := fileops.SyncArgs(opts.Host, fileops.SyncOptions{
		Source: local, Destination: "rhost-mirror-placeholder",
		DryRun: opts.DryRun, Delete: opts.Delete, Excludes: opts.Excludes,
	}, a.SSH.SSHOptions())
	if err != nil {
		return FsSyncResult{}, transferValidation(err)
	}
	source := fileops.RemoteSpec(opts.Host, strings.TrimRight(opts.Remote, "/")+"/")
	args[len(args)-2], args[len(args)-1] = source, local
	if opts.Checksum {
		args = append([]string{"--checksum"}, args...)
	}

	res, err := a.runTool(ctx, a.Transfers.RsyncBin, args, opts.timeout())
	if err != nil {
		return FsSyncResult{}, missingTool(errs.TransferFailed, "rsync", err)
	}
	if res.ExitCode != 0 {
		return FsSyncResult{}, mapTransferErr(res, "rsync could not mirror the directory")
	}
	changes, notes := fileops.ParseChanges(string(res.Stdout))
	mux = mux && a.SSH.MasterAlive(ctx, opts.Host)
	out := FsSyncResult{
		Source: source, Destination: local,
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
