//go:build !unix

package fileops

import "os/exec"

// configureProcessGroup is a no-op on platforms without Unix process groups. The
// default per-process cancellation still applies, and Cmd.WaitDelay (set in Run)
// bounds how long a timed-out transfer can block on a child holding the pipes.
func configureProcessGroup(cmd *exec.Cmd) {}
