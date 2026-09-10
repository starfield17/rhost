package cli

import (
	"fmt"
	"os"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/output"
)

// emitFailure renders an adapter failure and sets the process exit code.
func emitFailure(op, host string, aerr *errs.Error, code int) {
	if jsonFlag {
		_ = output.Failure(op, host, nil, aerr).Write(os.Stdout)
	} else {
		fmt.Fprintf(os.Stderr, "rhost: %s: %s\n", aerr.Code, aerr.Message)
	}
	exitCode = code
}
