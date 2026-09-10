package app

import (
	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/host"
)

// Hosts lists configured SSH host aliases. It does not require network access.
func (a *App) Hosts() ([]host.Info, *errs.Error) {
	infos, err := host.Aliases()
	if err != nil {
		return nil, errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
	}
	return infos, nil
}
