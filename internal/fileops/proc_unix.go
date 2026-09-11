//go:build unix

package fileops

import (
	"os/exec"
	"syscall"
)

// configureProcessGroup runs the tool in its own process group and arranges for
// cancellation to kill that whole group, not just the tool's top process.
//
// This is what makes a transfer timeout actually stop the transfer. A tool can
// fork a child that inherits stdout/stderr (a wrapper script, a helper); killing
// only the direct child leaves that child holding the pipes, and Wait then blocks
// on them until the child exits — the timeout would abandon the transfer rather
// than stop it. Measured: `sleep 5; exit 0` hung for the full 5s on Linux (dash
// forks the sleep) while passing on macOS (bash exec'd it), so this cannot rely
// on the shell's behaviour.
func configureProcessGroup(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.Cancel = func() error {
		if cmd.Process == nil {
			return nil
		}
		// A negative pid targets the process group. Setpgid made the child its own
		// group leader, so its pgid equals its pid. If the group is already gone,
		// fall back to the default per-process kill so the error semantics match.
		if err := syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL); err != nil {
			return cmd.Process.Kill()
		}
		return nil
	}
}
