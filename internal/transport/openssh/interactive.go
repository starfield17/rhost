package openssh

import (
	"context"
	"os"
	"os/exec"

	"github.com/starfield17/rhost/internal/config"
)

// RunInteractive runs remoteCmd with an allocated TTY and connects the local
// process's stdio to it. It is used for human-facing commands such as
// `session attach`.
func (c *Client) RunInteractive(ctx context.Context, target, remoteCmd string) error {
	if err := config.EnsureControlDir(); err != nil {
		return err
	}
	args := append(c.options(), "-t", "--", target, remoteCmd)
	cmd := exec.CommandContext(ctx, c.cfg.SSHBin, args...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}
