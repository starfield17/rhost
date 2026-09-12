package app

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/fileops"
	"github.com/starfield17/rhost/internal/shell"
)

// RemoteFile runs one structured request against the embedded remote helper and
// returns its decoded response.
//
// The helper program is sent as base64 inside `python3 -c`, and the request goes
// in on stdin, so a path or pattern never has to survive a shell parser at
// either end. The response is a single JSON document: either the operation's
// result, or `{"error": <code>, "message": ...}` where the code belongs to the
// same taxonomy the rest of rhost emits.
//
// cap is the helper's own budget; the transport limit is derived from it so a
// misbehaving helper cannot make this process drain an unbounded stream. The
// derivation leaves room for the JSON envelope around the payload, which is why
// it is generous rather than exact.
func (a *App) RemoteFile(ctx context.Context, host string, request map[string]interface{}, timeout time.Duration) (map[string]interface{}, *errs.Error) {
	if path, ok := request["path"].(string); ok {
		if err := fileops.ValidateRemotePath(path); err != nil {
			return nil, errs.New(errs.ConfigInvalid, err.Error(), false)
		}
	}
	if cap, ok := request["max_bytes"]; ok {
		n, valid := cap.(int)
		if !valid || n <= 0 || n > 8*1024*1024 {
			return nil, errs.New(errs.ConfigInvalid, "max_bytes must be between 1 and 8388608", false)
		}
	}
	payload, err := json.Marshal(request)
	if err != nil {
		return nil, errs.New(errs.ConfigInvalid, err.Error(), false)
	}
	program := "import base64; exec(base64.b64decode(\"" +
		base64.StdEncoding.EncodeToString([]byte(fileops.RemoteProgram)) + "\"))"
	command := "command -v python3 >/dev/null 2>&1 || exit 127; "
	command += "python3 -c " + shell.Quote(program)

	res, e := a.Execute(ctx, ExecOptions{
		Host:           host,
		Command:        command,
		Timeout:        timeout,
		Stdin:          payload,
		MaxOutputBytes: helperOutputBudget(request),
	})
	if e != nil {
		return nil, e
	}
	if res.ExitCode == 127 {
		return nil, errs.New(errs.RemoteDependencyMissing,
			"this operation needs remote python3; rhost reports it, it does not install it", false)
	}
	var out map[string]interface{}
	if err := json.Unmarshal([]byte(res.Stdout), &out); err != nil {
		return nil, errs.New(errs.Internal,
			"remote file helper returned no JSON: "+clip(firstLine(res.Stderr), 300), false)
	}
	code, ok := out["error"].(string)
	if !ok {
		return out, nil
	}
	// The helper runs on someone else's machine: trust its codes only because
	// they are a closed list, never an arbitrary string it could invent.
	if !errs.KnownCode(code) {
		return out, errs.New(errs.Internal, "unknown remote file error "+code, false)
	}
	message, _ := out["message"].(string)
	return out, errs.New(errs.Code(code), message, false)
}

// helperOutputBudget leaves room for the JSON response around a bounded file
// payload while still limiting a misbehaving helper.
func helperOutputBudget(request map[string]interface{}) int {
	maxBytes, _ := request["max_bytes"].(int)
	if maxBytes <= 0 {
		maxBytes = 256 * 1024
	}
	const overhead = 64 * 1024
	maxInt := int(^uint(0) >> 1)
	if maxBytes > (maxInt-overhead)/2 {
		return maxInt
	}
	return 2*maxBytes + overhead
}
