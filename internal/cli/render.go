package cli

import (
	"fmt"
	"os"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

// emitFailure renders an adapter failure and sets the process exit code.
//
// Every adapter failure exits 255 (timeouts 124) so that a status in 0-254 is
// always the *remote* command's own, regardless of which subcommand ran.
func emitFailure(op, host string, aerr *errs.Error) {
	if jsonFlag {
		_ = output.Failure(op, host, nil, aerr).Write(os.Stdout)
	} else {
		fmt.Fprintf(os.Stderr, "rhost: %s: %s\n", aerr.Code, aerr.Message)
	}
	exitCode = adapterExitCode(aerr)
}

// configErr wraps a plain error as a CONFIG_INVALID adapter error.
func configErr(err error) *errs.Error {
	return errs.Wrap(errs.ConfigInvalid, err.Error(), false, err)
}

// adapterExitCode is the single mapping from error taxonomy to process status.
func adapterExitCode(aerr *errs.Error) int {
	if aerr != nil && aerr.Code == errs.RemoteCommandTimeout {
		return 124
	}
	return 255
}
