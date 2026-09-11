// Package errs defines rhost's stable, machine-readable error taxonomy.
//
// The codes are a public compatibility surface: agents branch on Error.Code,
// never on English message text. Keep this list small and stable, and add new
// codes deliberately.
package errs

import (
	"fmt"
	"sort"
)

// Code identifies a class of adapter failure. It matches the candidate
// taxonomy in docs/ARCHITECTURE.md §33.
type Code string

const (
	// UsageError is emitted when argument/flag parsing fails, so that even the
	// usage path has a machine-readable form (AGENTS.md §6).
	UsageError              Code = "USAGE_ERROR"
	ConfigInvalid           Code = "CONFIG_INVALID"
	HostUnknown             Code = "HOST_UNKNOWN"
	SSHUnreachable          Code = "SSH_UNREACHABLE"
	SSHAuthFailed           Code = "SSH_AUTH_FAILED"
	HostKeyFailed           Code = "HOST_KEY_FAILED"
	RemoteDependencyMissing Code = "REMOTE_DEPENDENCY_MISSING"
	RemoteCommandTimeout    Code = "REMOTE_COMMAND_TIMEOUT"
	SessionNotFound         Code = "SESSION_NOT_FOUND"
	SessionUnhealthy        Code = "SESSION_UNHEALTHY"
	JobNotFound             Code = "JOB_NOT_FOUND"
	JobStateUnknown         Code = "JOB_STATE_UNKNOWN"
	TransferFailed          Code = "TRANSFER_FAILED"
	SyncRejected            Code = "SYNC_REJECTED"
	UnsupportedRemoteOS     Code = "UNSUPPORTED_REMOTE_OS"
	Internal                Code = "INTERNAL"
	FileConflict            Code = "FILE_CONFLICT"
	FileTooLarge            Code = "FILE_TOO_LARGE"
	InvalidText             Code = "INVALID_TEXT"
	SearchFailed            Code = "SEARCH_FAILED"
	TunnelFailed            Code = "TUNNEL_FAILED"
	// Codes owned by the remote file helper (internal/fileops/remote.py). They
	// describe what the helper refused, because only the remote side can tell
	// whether a path exists or a content hash still matches.
	FileNotFound  Code = "FILE_NOT_FOUND"
	InvalidTarget Code = "INVALID_TARGET"
	InvalidPatch  Code = "INVALID_PATCH"
	// TunnelNotFound names a record this machine no longer has.
	TunnelNotFound Code = "TUNNEL_NOT_FOUND"
)

// allCodes is the closed set an agent may branch on. Anything outside it is
// reported as INTERNAL: a code is a contract, and a string that arrives over the
// wire from another machine does not get to create one (AGENTS.md §6). The
// schema test compares this list with schemas/result-v1.schema.json.
var allCodes = []Code{
	UsageError, ConfigInvalid, HostUnknown, SSHUnreachable, SSHAuthFailed,
	HostKeyFailed, RemoteDependencyMissing, RemoteCommandTimeout, SessionNotFound,
	SessionUnhealthy, JobNotFound, JobStateUnknown, TransferFailed, SyncRejected,
	UnsupportedRemoteOS, Internal, FileConflict, FileTooLarge, InvalidText,
	SearchFailed, TunnelFailed, FileNotFound, InvalidTarget, InvalidPatch,
	TunnelNotFound,
}

var codeSet = func() map[Code]bool {
	m := make(map[Code]bool, len(allCodes))
	for _, c := range allCodes {
		m[c] = true
	}
	return m
}()

// Codes returns every published code, sorted, for contract checks.
func Codes() []string {
	out := make([]string, 0, len(allCodes))
	for _, c := range allCodes {
		out = append(out, string(c))
	}
	sort.Strings(out)
	return out
}

// KnownCode reports whether s is part of the published taxonomy.
func KnownCode(s string) bool { return codeSet[Code(s)] }

// Error is an adapter failure with a stable code. It is the only error type the
// application layer returns across the CLI boundary.
type Error struct {
	Code      Code
	Message   string
	Retryable bool
	cause     error
}

// New builds an Error without an underlying cause.
func New(code Code, message string, retryable bool) *Error {
	return &Error{Code: code, Message: message, Retryable: retryable}
}

// Wrap builds an Error that retains an underlying cause for diagnostics.
func Wrap(code Code, message string, retryable bool, cause error) *Error {
	return &Error{Code: code, Message: message, Retryable: retryable, cause: cause}
}

func (e *Error) Error() string { return e.Message }

func (e *Error) Unwrap() error { return e.cause }

// From coerces any error into an *Error, defaulting to Internal. A nil input
// returns nil so callers can write `From(maybeErr)`.
func From(err error) *Error {
	if err == nil {
		return nil
	}
	if e, ok := err.(*Error); ok {
		return e
	}
	return Wrap(Internal, err.Error(), false, err)
}

// Is reports whether err is an *Error with the given code.
func Is(err error, code Code) bool {
	e, ok := err.(*Error)
	return ok && e.Code == code
}

// String is a convenience for logging.
func (e *Error) String() string {
	return fmt.Sprintf("%s: %s", e.Code, e.Message)
}
