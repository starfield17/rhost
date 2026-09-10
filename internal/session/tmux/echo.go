package tmux

import "github.com/starfield17/rhost/internal/shell"

// EchoScript enables or disables terminal echo in a session's pane. rhost keeps
// echo disabled so agent-captured output is clean; `session attach` turns it on
// for the duration of a human attach and back off afterwards.
func EchoScript(nameOrID string, on bool) string {
	cmd := "stty -echo 2>/dev/null"
	if on {
		cmd = "stty echo 2>/dev/null"
	}
	return basePreamble + resolveFunc + tmuxPreflight +
		"RHOST_RESOLVE " + shell.Quote(nameOrID) + " || { echo RHOST_ERR=nosession; exit 0; }\n" +
		"tmux send-keys -t \"$RHOST_TMUX:0.0\" " + shell.Quote(cmd) + " Enter\n" +
		"echo RHOST_OK=echo\n"
}
