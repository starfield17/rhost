// Package app owns rhost's use-case semantics. It depends on transport
// capabilities and returns plain models plus *errs.Error; it performs no CLI
// parsing and no terminal formatting (docs/ARCHITECTURE.md).
package app

import (
	"context"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/fileops"
	"github.com/starfield17/rhost/internal/transport/openssh"
)

type ConnectionResult = openssh.ConnectionStatus

func (a *App) ConnectionStatus(ctx context.Context, host string) ConnectionResult {
	return a.SSH.ConnectionStatus(ctx, host)
}

func (a *App) ConnectionReset(ctx context.Context, host string) (ConnectionResult, *errs.Error) {
	res, err := a.SSH.ResetConnection(ctx, host)
	if err != nil {
		return res, errs.Wrap(errs.SSHControlFailed,
			"could not stop the shared SSH master: "+res.Diagnostic, false, err)
	}
	return res, nil
}

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
