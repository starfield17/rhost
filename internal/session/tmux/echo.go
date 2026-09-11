package tmux

import "github.com/starfield17/rhost/internal/shell"

// EchoScript enables or disables terminal echo in a session's pane. rhost keeps
// echo disabled so agent-captured output is clean; `session attach` turns it on
// for the duration of a human attach and back off afterwards.
//
// The flag is a property of the pane's pty, so it is set on the pty: typing
// `stty …` into the pane would send those keystrokes to whatever owns the
// foreground — a REPL, a debugger, an editor — and change nothing about the
// terminal it was meant to change (docs/ARCHITECTURE.md §16).
func EchoScript(nameOrID string, on bool) string {
	flag := "-echo"
	if on {
		flag = "echo"
	}
	return basePreamble + resolveFunc + tmuxPreflight +
		"RHOST_RESOLVE " + shell.Quote(nameOrID) + " || { echo RHOST_ERR=nosession; exit 0; }\n" +
		"tmux has-session -t \"$RHOST_TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n" +
		"TTY=$(tmux display-message -p -t \"$RHOST_TMUX:0.0\" '#{pane_tty}' 2>/dev/null)\n" +
		"[ -n \"$TTY\" ] || { echo RHOST_ERR=notty; exit 0; }\n" +
		"stty " + flag + " < \"$TTY\" 2>/dev/null || { echo RHOST_ERR=notty; exit 0; }\n" +
		"echo RHOST_OK=echo\n"
}
