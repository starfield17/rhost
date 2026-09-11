package fileops

import (
	"context"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode"

	"github.com/starfield17/rhost/internal/shell"
)

// SafeRsyncPaths makes the remote path of an rsync command survive *two* shells.
//
// rsync passes the `host:path` spec through the remote login shell, so a path with
// a space, a bracket or a `$` in it is otherwise re-parsed there — the same class
// of bug that `fs put` closes for scp. There are two ways to fix it and they are
// mutually exclusive:
//
//   - GNU rsync 3 and later have --protect-args, which stops the local tool from
//     splitting or re-quoting anything;
//   - openrsync (what macOS ships) and rsync 2.x do not know that flag, and would
//     fail the whole transfer on an unrecognised option, so the path is quoted for
//     the remote shell by hand instead.
//
// Both routes are only needed for a path that is not plain: `[A-Za-z0-9._~/-]`
// characters are inert in both shells, and the overwhelming majority of paths are
// made of them, which keeps the common case free of an extra version probe.
func (r *Runner) SafeRsyncPaths(ctx context.Context, args []string) ([]string, error) {
	out := append([]string(nil), args...)
	needy := -1
	// Only the last two arguments can be endpoints; everything before them is
	// rsync's own flags and the -e string.
	for i := len(out) - 2; i < len(out); i++ {
		if i < 0 {
			continue // an argv too short to hold a remote spec
		}
		_, path, ok := SplitRemoteSpec(out[i])
		if !ok {
			continue
		}
		if pathNeedsQuoting(path) {
			needy = i
			break
		}
	}
	if needy < 0 {
		return out, nil
	}
	if protectArgsSupported(ctx, r) {
		return append([]string{"--protect-args"}, out...), nil
	}
	host, path, _ := SplitRemoteSpec(out[needy])
	out[needy] = RemoteSpec(host, shell.PathQuote(path))
	return out, nil
}

// pathNeedsQuoting is true for anything that is not inert in a remote shell.
// Dotfiles (`.`), a leading `~`, and characters like `*` or `$` all need it; so do
// parentheses and quotes, which is exactly why the quoting is rhost's job.
func pathNeedsQuoting(path string) bool {
	return strings.IndexFunc(path, func(c rune) bool {
		return !(unicode.IsLetter(c) || unicode.IsDigit(c) ||
			strings.ContainsRune("/._~-", c))
	}) >= 0
}

// gnuRsyncMajor is the version banner of the local rsync. openrsync answers with
// its own line, which is why the probe requires the word `version` to follow the
// word `rsync` directly; and a banner is matched on its *first* line, because
// later lines list features and protocol numbers that are not the tool's name.
var gnuRsyncMajor = regexp.MustCompile(`(?i)^rsync\s+version\s+(\d+)`)

// protectArgsSupported asks the local rsync what it is. A probe that fails at all
// — no banner, no pattern — is answered conservatively: quoting the path by hand
// works on every rsync, while guessing --protect-args on a tool that lacks it
// breaks a transfer that would otherwise have run.
//
// The bound is deliberately generous for a `--version`: this runs on the way to a
// transfer that will take seconds anyway, and the cost of cutting the probe short
// is silently taking the conservative route — correct, but with a hand-quoted path
// the caller did not ask for.
const rsyncVersionProbeTimeout = 15 * time.Second

func protectArgsSupported(ctx context.Context, r *Runner) bool {
	res, err := r.Run(ctx, r.RsyncBin, []string{"--version"}, rsyncVersionProbeTimeout)
	if err != nil || res.ExitCode != 0 {
		return false
	}
	return protectArgsDecision(string(res.Stdout))
}

// protectArgsDecision is the whole parsing question: given a `--version` banner,
// does this tool understand --protect-args? Split out so every real-world banner is
// pinned by a table instead of by how fast a machine can spawn a shell.
func protectArgsDecision(stdout string) bool {
	first := strings.TrimSpace(strings.SplitN(stdout, "\n", 2)[0])
	if strings.Contains(strings.ToLower(first), "openrsync") {
		return false
	}
	m := gnuRsyncMajor.FindStringSubmatch(first)
	if m == nil {
		return false
	}
	major, err := strconv.Atoi(m[1])
	return err == nil && major >= 3
}
