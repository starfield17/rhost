package app

import (
	"context"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/errs"
)

// DoctorResult is the capability snapshot produced by Doctor.
type DoctorResult struct {
	Host             string          `json:"host"`
	Online           bool            `json:"online"`
	OS               string          `json:"os"`
	Kernel           string          `json:"kernel"`
	Arch             string          `json:"arch"`
	User             string          `json:"user"`
	Home             string          `json:"home"`
	LoginShell       string          `json:"login_shell"`
	StateDir         string          `json:"state_dir"`
	StateDirWritable bool            `json:"state_dir_writable"`
	WSL              bool            `json:"wsl"`
	Capabilities     map[string]bool `json:"capabilities"`
}

// doctorProbe is a read-only POSIX script that prints `key=value` lines. It runs
// through the normal exec wrapper (bash -lc), so a success also proves batch
// auth, bash, setsid, and writing the completion marker all work.
const doctorProbe = `em() { printf '%s=%s\n' "$1" "$2"; }
em os "$( (. /etc/os-release 2>/dev/null; printf '%s' "${PRETTY_NAME:-unknown}") )"
em kernel "$(uname -r 2>/dev/null)"
em arch "$(uname -m 2>/dev/null)"
em user "$(id -un 2>/dev/null)"
em home "$HOME"
ls="$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)"
[ -n "$ls" ] || ls="${SHELL:-unknown}"
em login_shell "$ls"
for c in bash tmux setsid ps rsync sha256sum base64 stty flock python3 realpath; do
  if command -v "$c" >/dev/null 2>&1; then em "have_$c" yes; else em "have_$c" no; fi
done
rd="${RHOST_REMOTE_STATE:-$HOME/.local/state/rhost}"
if mkdir -p "$rd" 2>/dev/null && [ -w "$rd" ]; then em state_dir_writable yes; else em state_dir_writable no; fi
em state_dir "$rd"
if grep -qi microsoft /proc/version 2>/dev/null; then em wsl yes; else em wsl no; fi
`

// Doctor probes a host's capabilities without assuming them.
func (a *App) Doctor(ctx context.Context, host string, timeout time.Duration) (DoctorResult, *errs.Error) {
	res, aerr := a.Execute(ctx, ExecOptions{
		Host:    host,
		Command: doctorProbe,
		Timeout: timeout,
	})
	if aerr != nil {
		return DoctorResult{Host: host, Online: false}, aerr
	}

	kv := parseKV(res.Stdout)
	caps := map[string]bool{}
	for k, v := range kv {
		if name, ok := strings.CutPrefix(k, "have_"); ok {
			caps[name] = v == "yes"
		}
	}
	return DoctorResult{
		Host:             host,
		Online:           true,
		OS:               kv["os"],
		Kernel:           kv["kernel"],
		Arch:             kv["arch"],
		User:             kv["user"],
		Home:             kv["home"],
		LoginShell:       kv["login_shell"],
		StateDir:         kv["state_dir"],
		StateDirWritable: kv["state_dir_writable"] == "yes",
		WSL:              kv["wsl"] == "yes",
		Capabilities:     caps,
	}, nil
}

// parseKV parses `key=value` lines, splitting on the first '='.
func parseKV(s string) map[string]string {
	m := map[string]string{}
	for _, line := range strings.Split(s, "\n") {
		line = strings.TrimRight(line, "\r")
		if line == "" {
			continue
		}
		i := strings.IndexByte(line, '=')
		if i <= 0 {
			continue
		}
		m[line[:i]] = line[i+1:]
	}
	return m
}
