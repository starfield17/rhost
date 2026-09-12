// Command rhost runs ordinary shell commands in an SSH-reachable remote context.
// It also exposes the few explicit operations that need persistence or protected
// cross-machine state.
//
// This file only wires the CLI together; all remote-control logic lives under
// internal/.
package main

import (
	"os"

	"github.com/starfield17/rhost/internal/cli"
)

func main() {
	os.Exit(cli.Run())
}
