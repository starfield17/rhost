// Package app owns rhost's use-case semantics. It depends on transport
// capabilities and returns plain models plus *errs.Error; it performs no CLI
// parsing and no terminal formatting (docs/ARCHITECTURE.md §3).
package app

import (
	"time"

	"github.com/starfield17/rhost/internal/transport/openssh"
)

// App bundles the dependencies shared by all use-cases.
type App struct {
	SSH            *openssh.Client
	DefaultTimeout time.Duration
}

// NewDefault builds an App backed by the system OpenSSH client.
func NewDefault() *App {
	return &App{
		SSH:            openssh.New(openssh.DefaultConfig()),
		DefaultTimeout: 60 * time.Second,
	}
}
