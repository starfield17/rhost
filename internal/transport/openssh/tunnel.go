package openssh

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/starfield17/rhost/internal/config"
)

// Tunnel is one port forward owned by a *dedicated* OpenSSH master.
//
// This is where rhost's persistence invariant is easiest to get wrong, so the
// ownership is explicit: the CLI process that opened a tunnel owns nothing. The
// forward is held by its own `ssh -o ControlMaster=yes -o ControlPersist=yes -fNT`
// process, and the record on disk is only how a later process finds that master
// again (AGENTS.md §4). A tunnel therefore survives a CLI exit and a client
// reboot does not: nothing here is a service, and `rhost tunnel list` is the way
// to discover what is still up.
//
// Each tunnel gets its own socket, deliberately not the shared one: `tunnel close`
// must be able to stop one forward without dropping the multiplexed connection
// that exec, session and job traffic are using.
type Tunnel struct {
	ID          string `json:"id"`
	Host        string `json:"host"`
	Kind        string `json:"kind"`
	Listen      string `json:"listen"`
	Destination string `json:"destination,omitempty"`
	Status      string `json:"status"`
}

// ErrNoTunnel is returned when a tunnel record does not exist on this machine. It
// is a different answer from "the master died", which is what `stale` means.
var ErrNoTunnel = errors.New("no such tunnel")

// ErrInvalidTunnel marks a forward rejected entirely from local arguments.
// Callers map it to CONFIG_INVALID rather than a retryable transport failure.
var ErrInvalidTunnel = errors.New("invalid tunnel configuration")

// Status values are stable names, not prose: `alive` means the dedicated master
// answered, `stale` means the record outlived it.
const (
	TunnelAlive = "alive"
	TunnelStale = "stale"
)

var tunnelID = regexp.MustCompile(`^t_[a-f0-9]{32}$`)

func tunnelRoot() string { return filepath.Join(config.StateDir(), "tunnels") }

// tunnelSocket is the ControlPath of one tunnel's own master. It lives in the
// control dir, whose length is already budgeted for OpenSSH's socket limit.
func tunnelSocket(id string) string { return filepath.Join(config.ControlDir(), id) }

// ListTunnels reads the local records and asks each master whether it is still
// there. A stale record is reported, not deleted: something else may still be
// using the forward's slot, and quietly dropping the only pointer to it would be
// the worse guess.
func (c *Client) ListTunnels(ctx context.Context) ([]Tunnel, error) {
	entries, err := os.ReadDir(tunnelRoot())
	if os.IsNotExist(err) {
		return []Tunnel{}, nil
	}
	if err != nil {
		return nil, err
	}
	out := []Tunnel{}
	for _, entry := range entries {
		id := strings.TrimSuffix(entry.Name(), ".json")
		if !tunnelID.MatchString(id) || entry.IsDir() {
			continue
		}
		t, err := readTunnel(id)
		if err != nil {
			return nil, err
		}
		if c.masterError(ctx, t, "check") != nil {
			t.Status = TunnelStale
		}
		out = append(out, t)
	}
	return out, nil
}

// OpenTunnel starts a dedicated master carrying exactly one forward.
//
// The two guards are the point of the command:
//
//  1. a bind address that is not loopback is a change to who else can reach the
//     remote network, so it takes an explicit --allow-exposure rather than a
//     default someone has to remember to change;
//  2. ExitOnForwardFailure plus a `-O check` before the record is written means
//     `status: alive` is a fact that was observed, not a hope that ssh returned
//     quickly enough.
//
// The remote sshd still decides whether a reverse bind is permitted
// (GatewayPorts, AllowTcpForwarding): rhost reports what OpenSSH said and never
// works around it.
func (c *Client) OpenTunnel(ctx context.Context, host, kind, listen, destination string, expose bool) (Tunnel, error) {
	t := Tunnel{Host: host, Kind: kind, Listen: listen, Destination: destination}

	flag, err := forwardFlag(kind)
	if err != nil {
		return t, err
	}
	spec, err := forwardSpec(kind, listen, destination)
	if err != nil {
		return t, err
	}
	if err := checkBind(listen, expose); err != nil {
		return t, err
	}

	nonce, err := NewNonce()
	if err != nil {
		return t, err
	}
	t.ID = "t_" + nonce
	t.Status = TunnelAlive

	if err := config.EnsureControlDir(); err != nil {
		return t, err
	}
	if err := os.MkdirAll(tunnelRoot(), 0o700); err != nil {
		return t, err
	}

	// rhost's own options come second: OpenSSH keeps the first value it is given
	// for a parameter, so the tunnel's ControlPath and ControlMaster win over the
	// shared socket and the multiplexed auto-master. That is what makes this
	// connection independent of every other command.
	args := []string{
		"-o", "ControlPath=" + tunnelSocket(t.ID),
		"-o", "ControlMaster=yes",
		"-o", "ControlPersist=yes",
		"-o", "ExitOnForwardFailure=yes",
		"-o", "ServerAliveInterval=15",
		"-o", "ServerAliveCountMax=3",
	}
	args = append(args, c.options()...)
	args = append(args, "-fNT", flag, spec, "--", host)

	runner, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	if out, err := exec.CommandContext(runner, c.cfg.SSHBin, args...).CombinedOutput(); err != nil {
		_ = c.masterError(context.Background(), t, "exit")
		return t, fmt.Errorf("cannot open tunnel: %s", strings.TrimSpace(string(out)))
	}
	check, cancel2 := context.WithTimeout(ctx, 10*time.Second)
	defer cancel2()
	if err := c.masterError(check, t, "check"); err != nil {
		_ = c.masterError(context.Background(), t, "exit")
		return t, fmt.Errorf("tunnel started but its OpenSSH master is not answering: %w", err)
	}

	data, err := json.Marshal(t)
	if err != nil {
		return t, err
	}
	if err := os.WriteFile(filepath.Join(tunnelRoot(), t.ID+".json"), data, 0o600); err != nil {
		_ = c.masterError(context.Background(), t, "exit")
		return t, err
	}
	return t, nil
}

// ValidateTunnel checks a forward without creating directories, starting SSH or
// writing a record. It lets the CLI classify deterministic argument errors
// before treating later OpenSSH failures as retryable transport failures.
func ValidateTunnel(kind, listen, destination string, expose bool) error {
	if _, err := forwardFlag(kind); err != nil {
		return fmt.Errorf("%w: %v", ErrInvalidTunnel, err)
	}
	if _, err := forwardSpec(kind, listen, destination); err != nil {
		return fmt.Errorf("%w: %v", ErrInvalidTunnel, err)
	}
	if err := checkBind(listen, expose); err != nil {
		return fmt.Errorf("%w: %v", ErrInvalidTunnel, err)
	}
	return nil
}

// CloseTunnel stops exactly one tunnel's master and removes its record. A record
// whose master is already gone is still closed: the point is to leave nothing
// half-remembered behind.
func (c *Client) CloseTunnel(ctx context.Context, id string) error {
	t, err := readTunnel(id)
	if err != nil {
		return err
	}
	if c.masterError(ctx, t, "check") == nil {
		if err := c.masterError(ctx, t, "exit"); err != nil {
			return err
		}
	}
	return os.Remove(filepath.Join(tunnelRoot(), id+".json"))
}

// readTunnel loads one record and checks that the file is named after the tunnel
// it describes, so a stray or hand-edited file cannot make rhost signal some other
// process's socket.
func readTunnel(id string) (Tunnel, error) {
	if !tunnelID.MatchString(id) {
		return Tunnel{}, fmt.Errorf("%w: %s", ErrNoTunnel, id)
	}
	data, err := os.ReadFile(filepath.Join(tunnelRoot(), id+".json"))
	if os.IsNotExist(err) {
		return Tunnel{}, fmt.Errorf("%w: %s", ErrNoTunnel, id)
	}
	if err != nil {
		return Tunnel{}, err
	}
	var t Tunnel
	if err := json.Unmarshal(data, &t); err != nil {
		return Tunnel{}, fmt.Errorf("tunnel record %s is unreadable: %w", id, err)
	}
	if t.ID != id {
		return Tunnel{}, fmt.Errorf("tunnel record %s describes %s", id, t.ID)
	}
	return t, nil
}

// masterError runs one `ssh -O` control request against a tunnel's own socket.
func (c *Client) masterError(ctx context.Context, t Tunnel, action string) error {
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	out, err := exec.CommandContext(ctx, c.cfg.SSHBin, "-S", tunnelSocket(t.ID),
		"-O", action, "--", t.Host).CombinedOutput()
	if err != nil {
		return fmt.Errorf("ssh -O %s: %s", action, strings.TrimSpace(string(out)))
	}
	return nil
}

// forwardFlag maps the three supported kinds onto ssh's own flags, which is the
// whole of rhost's tunnel vocabulary: local, reverse, dynamic.
func forwardFlag(kind string) (string, error) {
	switch kind {
	case "local":
		return "-L", nil
	case "reverse":
		return "-R", nil
	case "socks":
		return "-D", nil
	default:
		return "", fmt.Errorf("kind must be local, reverse or socks, not %q", kind)
	}
}

// forwardSpec builds ssh's forward argument and rejects the shapes that would
// silently mean something else: an empty host, and port 0, which is a request for
// "any port" that rhost could never report back.
func forwardSpec(kind, listen, destination string) (string, error) {
	_, port, err := net.SplitHostPort(listen)
	if err != nil {
		return "", fmt.Errorf("--listen must be address:port, not %q", listen)
	}
	if port == "0" {
		return "", fmt.Errorf("--listen port must be explicit; rhost cannot report a port it did not choose")
	}
	spec := listen
	if kind == "socks" {
		if destination != "" {
			return "", fmt.Errorf("--destination is not used for a socks forward")
		}
		return spec, nil
	}
	if _, _, err := net.SplitHostPort(destination); err != nil {
		return "", fmt.Errorf("--destination must be host:port for a %s forward", kind)
	}
	return spec + ":" + destination, nil
}

// checkBind is the exposure guard. `localhost` is accepted by name because that is
// what a person types; an explicit 127.0.0.1 or ::1 is the same promise.
func checkBind(listen string, expose bool) error {
	addr, _, err := net.SplitHostPort(listen)
	if err != nil {
		return err
	}
	if addr == "localhost" {
		return nil
	}
	if ip := net.ParseIP(addr); ip != nil && ip.IsLoopback() {
		// 127.0.0.53 and ::1 are as local as 127.0.0.1: what matters is that the
		// kernel will not accept the connection from another machine.
		return nil
	}
	if expose {
		return nil
	}
	return fmt.Errorf("binding %s is reachable from elsewhere; pass --allow-exposure to mean it", listen)
}
