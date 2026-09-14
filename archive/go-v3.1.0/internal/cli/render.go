package cli

import (
	"fmt"
	"os"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

var outputFailed bool

// writeEnvelope never retries on stdout: a failed write may have delivered a
// partial document, and the remote operation may already have completed.
func writeEnvelope(e output.Envelope) {
	if err := e.Write(os.Stdout); err != nil {
		outputFailed = true
		fmt.Fprintf(os.Stderr, "rhost: %s: result delivery failed (operation may have completed): %v\n", errs.OutputWriteFailed, err)
	}
}

// emitFailure renders an adapter failure and sets the process exit code.
//
// Every adapter failure exits 255 (timeouts 124) so that a status in 0-254 is
// always the *remote* command's own, regardless of which subcommand ran.
func emitFailure(op, host string, aerr *errs.Error) {
	if jsonFlag {
		writeEnvelope(output.Failure(op, host, nil, aerr))
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
