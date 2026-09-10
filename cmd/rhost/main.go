// Command rhost is a general-purpose remote-host adapter for coding agents and
// humans. It orchestrates the system OpenSSH client so that an agent can treat
// an SSH-reachable machine as a reusable execution node.
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
