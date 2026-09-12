package app

import (
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/host"
)

// Hosts lists configured SSH host aliases. It does not require network access.
func (a *App) Hosts() (host.Discovery, *errs.Error) {
	result, err := host.Aliases()
	if err != nil {
		return host.Discovery{}, errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
	}
	return result, nil
}
