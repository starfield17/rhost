// Package app owns rhost's use-case semantics. It depends on transport
// capabilities and returns plain models plus *errs.Error; it performs no CLI
// parsing and no terminal formatting (docs/ARCHITECTURE.md §3).
package app

import (
	"time"

	"github.com/starfield17/rhost/internal/fileops"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

// App bundles the dependencies shared by all use-cases.
type App struct {
	SSH            *openssh.Client
	DefaultTimeout time.Duration
	// Transfers runs the local scp/rsync processes that `fs` is a thin semantic
	// layer over. It is a field so a test can point it at a stub binary.
	Transfers *fileops.Runner
}

// NewDefault builds an App backed by the system OpenSSH client.
func NewDefault() *App {
	return &App{
		SSH:            openssh.New(openssh.DefaultConfig()),
		DefaultTimeout: 60 * time.Second,
		Transfers:      fileops.NewRunner(),
	}
}
