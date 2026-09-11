// Package output defines rhost's agent-facing JSON envelope and small helpers
// for human rendering. The JSON document is a public compatibility surface
// (docs/ARCHITECTURE.md §32), so its shape is versioned.
package output

import (
	"encoding/json"
	"io"

	"github.com/starfield17/rhost/internal/errs"
)

// SchemaVersion is the version of the JSON result envelope. It is independent
// of the binary version and only changes on a breaking schema change. Keep in
// sync with schemas/result-v1.schema.json.
const SchemaVersion = 1

// ErrorPayload is the machine-readable error object embedded in an Envelope.
type ErrorPayload struct {
	Code      string `json:"code"`
	Message   string `json:"message"`
	Retryable bool   `json:"retryable"`
}

// Envelope is the single JSON document emitted by every agent-facing command.
type Envelope struct {
	SchemaVersion int           `json:"schema_version"`
	Operation     string        `json:"operation"`
	OK            bool          `json:"ok"`
	Host          string        `json:"host,omitempty"`
	Data          interface{}   `json:"data"`
	Error         *ErrorPayload `json:"error"`
}

// Success builds a successful envelope. data may be nil.
func Success(operation, host string, data interface{}) Envelope {
	return Envelope{
		SchemaVersion: SchemaVersion,
		Operation:     operation,
		OK:            true,
		Host:          host,
		Data:          data,
		Error:         nil,
	}
}

// ErrorOf renders one adapter error as the envelope's error object.
//
// It is exported because a command whose *aggregate* outcome is a success can
// still contain failed items — exec-many and fs batch report per-target results
// inside one document — and each of those needs the same machine-readable shape
// (AGENTS.md §6: an agent branches on error.code, never on text).
func ErrorOf(err *errs.Error) *ErrorPayload {
	if err == nil {
		return nil
	}
	return &ErrorPayload{Code: string(err.Code), Message: err.Message, Retryable: err.Retryable}
}

// Failure builds a failed envelope. data may be nil, or may carry partial
// information (for example partial stdout on a timeout).
func Failure(operation, host string, data interface{}, err *errs.Error) Envelope {
	return Envelope{
		SchemaVersion: SchemaVersion,
		Operation:     operation,
		OK:            false,
		Host:          host,
		Data:          data,
		Error:         ErrorOf(err),
	}
}

// Write encodes the envelope as a single line of JSON.
func (e Envelope) Write(w io.Writer) error {
	enc := json.NewEncoder(w)
	enc.SetEscapeHTML(false)
	return enc.Encode(e)
}
