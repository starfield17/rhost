package app

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"time"

	"github.com/starfield17/rhost/internal/errs"
	"github.com/starfield17/rhost/internal/fileops"
	"github.com/starfield17/rhost/internal/shell"
)

// remoteHelpers is what one request needs from the remote account beyond bash:
// python3 always, and rg for the two search operations. A missing helper is a
// capability answer (`REMOTE_DEPENDENCY_MISSING`), and doctor already reports it
// — rhost never installs anything to work around it (AGENTS.md §5).
var remoteHelpers = map[string]string{
	"grep": "rg",
	"glob": "rg",
}

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
	payload, err := json.Marshal(request)
	if err != nil {
		return nil, errs.New(errs.ConfigInvalid, err.Error(), false)
	}
	program := "import base64; exec(base64.b64decode(\"" +
		base64.StdEncoding.EncodeToString([]byte(fileops.RemoteProgram)) + "\"))"
	command := "command -v python3 >/dev/null 2>&1 || exit 127; "
	if helper, ok := remoteHelpers[fmt.Sprint(request["op"])]; ok {
		command += "command -v " + helper + " >/dev/null 2>&1 || exit 127; "
	}
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
		need := "python3"
		if helper, ok := remoteHelpers[fmt.Sprint(request["op"])]; ok {
			need += " and " + helper
		}
		return nil, errs.New(errs.RemoteDependencyMissing,
			"this operation needs remote "+need+"; rhost reports it, it does not install it", false)
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

// helperOutputBudget bounds the transport, not the helper: a search that hits
// its own cap stays far below this, so exceeding it means the helper itself
// misbehaved.
func helperOutputBudget(request map[string]interface{}) int {
	maxBytes, _ := request["max_bytes"].(int)
	if maxBytes <= 0 {
		maxBytes = 256 * 1024
	}
	return 2*maxBytes + 64*1024
}
