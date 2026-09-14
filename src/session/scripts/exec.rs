//! `exec`: one command, one completion boundary.

use super::shared::{
    FLOCK_PREFLIGHT, MARKER_SCAN_FUNC, RESOLVE_FUNC, SHELL_OF_FUNC, TMUX_PREFLIGHT, preamble,
};
use crate::base64;
use crate::session::DEFAULT_SHELL;
use crate::shell;
use std::time::Duration;

/// Runs one command in an existing session.
///
/// The result is accepted only when the pane emits a marker carrying this
/// invocation's token and a valid shell exit status. Writers are serialised with a
/// non-blocking `flock`: another writer holding it is reported immediately rather
/// than queued behind it. On its own timeout the helper interrupts the command and
/// reports whether the shell came back to a prompt.
pub fn exec(name_or_id: &str, command: &str, timeout: Duration, token: &str) -> String {
    let payload = format!("{{\n{command}\ncommand printf '\\033]133;R;{token};%d\\007' \"$?\"\n}}");
    let command_b64 = base64::encode(payload.as_bytes());
    let timeout_sec = timeout.as_secs().max(1);

    let mut out = String::new();
    out.push_str(&preamble());
    out.push_str(RESOLVE_FUNC);
    out.push_str(&format!(
        "RHOST_RESOLVE {} || {{ echo RHOST_ERR=nosession; exit 0; }}\n",
        shell::quote(name_or_id)
    ));
    out.push_str("echo \"RHOST_ID=$RHOST_ID\"\n");
    out.push_str(TMUX_PREFLIGHT);
    out.push_str(FLOCK_PREFLIGHT);
    out.push_str(
        "DIR=\"$RHOST_DIR\"; LOG=\"$DIR/pty.log\"; LOCK=\"$DIR/lock\"; TMUX=\"$RHOST_TMUX\"\n",
    );
    out.push_str("LOG=$(printf '%s' \"$LOG\" | sed 's#/$##')\n");
    out.push_str("exec 9>\"$LOCK\" || { echo RHOST_ERR=nosession; exit 0; }\n");
    out.push_str("flock -n 9 || { echo RHOST_ERR=locked; exit 0; }\n");
    out.push_str(
        "tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n",
    );
    out.push_str("BUF=rhost_cmd_$$\n");
    out.push_str("cleanup() { tmux delete-buffer -b \"$BUF\" 2>/dev/null; }\n");
    out.push_str("trap cleanup EXIT; trap 'exit 1' HUP INT TERM\n");
    // Everything below types into the pane's terminal, which is only safe while
    // the managed shell owns the foreground: after `session send --data
    // 'python3\n'` the pane is a REPL and a pasted command would be read by *it*.
    out.push_str(SHELL_OF_FUNC);
    out.push_str("want=$(RHOST_SHELL_OF \"$DIR/meta.json\" 2>/dev/null)\n");
    out.push_str("[ -n \"$want\" ] || want=");
    out.push_str(&shell::quote(DEFAULT_SHELL));
    out.push('\n');
    out.push_str(
        "fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n",
    );
    out.push_str("if [ -z \"$fg\" ]; then echo RHOST_ERR=unknownfg; exit 0; fi\n");
    out.push_str("if [ \"$fg\" != \"$want\" ]; then echo \"RHOST_FG=$fg\"; echo RHOST_ERR=busy; exit 0; fi\n");
    out.push_str(MARKER_SCAN_FUNC);
    out.push_str("pre=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n");
    out.push_str("TTY=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_tty}' 2>/dev/null)\n");
    out.push_str("[ -n \"$TTY\" ] || { echo RHOST_ERR=notty; exit 0; }\n");
    out.push_str("stty -echo < \"$TTY\" 2>/dev/null || { echo RHOST_ERR=notty; exit 0; }\n");
    out.push_str("dpat=$(printf '\\033]133;D;'); dlen=${#dpat}\n");
    // Wait for the pane to be idle: one Enter, then a prompt marker.
    out.push_str("tmux send-keys -t \"$TMUX:0.0\" Enter\n");
    out.push_str("floor=$((pre + 1)); scan=$floor; i=0; while [ \"$i\" -lt 50 ]; do rh_window && break; sleep 0.1; i=$((i+1)); done\n");
    out.push_str("[ \"$i\" -lt 50 ] || { echo RHOST_ERR=notready; exit 0; }\n");
    out.push_str(
        "fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n",
    );
    out.push_str("if [ -z \"$fg\" ]; then echo RHOST_ERR=unknownfg; exit 0; fi\n");
    out.push_str("if [ \"$fg\" != \"$want\" ]; then echo \"RHOST_FG=$fg\"; echo RHOST_ERR=busy; exit 0; fi\n");
    out.push_str("start=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n");
    // The command is pasted as one compound command, so it produces one marker.
    // The submitting newline is part of the paste: a separate `send-keys Enter`
    // can reach the pane before the paste, even inside one tmux command queue.
    out.push_str(&format!("cmd=$(printf '%s' '{command_b64}' | base64 -d)\n"));
    out.push_str("printf '%s\\n' \"$cmd\" | tmux load-buffer -b \"$BUF\" - \\; paste-buffer -d -b \"$BUF\" -t \"$TMUX:0.0\" 2>/dev/null || { echo RHOST_ERR=inputfailed; exit 0; }\n");
    out.push_str(&format!(
        "rpat=$(printf '\\033]133;R;{token};'); dpat=$rpat; dlen=${{#dpat}}\n"
    ));
    out.push_str(&format!(
        "SECONDS=0; found=0; died=0; floor=$((start + 1)); scan=$floor; while [ \"$SECONDS\" -lt {timeout_sec} ]; do rh_window && {{ found=1; break; }}; if ! tmux has-session -t \"$TMUX\" 2>/dev/null; then died=1; break; fi; if [ \"$SECONDS\" -ge 5 ]; then sleep 1; elif [ \"$SECONDS\" -ge 1 ]; then sleep 0.5; else sleep 0.1; fi; done\n"
    ));
    out.push_str("if [ \"$died\" = 1 ]; then echo RHOST_ERR=sessiondied; exit 0; fi\n");
    // A command that outlived its deadline is interrupted, and then the shell is
    // *proved* usable before the session is handed back: Ctrl-C followed by a
    // fresh command boundary. That proof is reported, never assumed — an
    // interrupt the pane ignored leaves a program holding the session.
    out.push_str("if [ \"$found\" != 1 ]; then\n");
    out.push_str("  tmux send-keys -t \"$TMUX:0.0\" C-c 2>/dev/null\n");
    out.push_str("  recovered=0; dpat=$(printf '\\033]133;D;'); dlen=${#dpat}; floor=$((scan + 1)); scan=$floor; i=0\n");
    out.push_str("  while [ \"$i\" -lt 50 ]; do rh_window && { recovered=1; break; }; sleep 0.1; i=$((i+1)); done\n");
    out.push_str(
        "  fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n",
    );
    out.push_str("  [ \"$fg\" = \"$want\" ] || recovered=0\n");
    out.push_str("  echo \"RHOST_RECOVERED=$recovered\"\n");
    out.push_str("  echo RHOST_ERR=timeout\n");
    out.push_str("  exit 0\n");
    out.push_str("fi\n");
    // Locate this invocation's marker, validate the status that follows *it*, and
    // emit the exact fields the parser expects.
    out.push_str("dline=$(tail -c +$((start+1)) \"$LOG\" | grep -aboF \"$rpat\" | head -1)\n");
    out.push_str("doff=${dline%%:*}\n");
    out.push_str("abs=$((start + doff))\n");
    out.push_str("seg=$(tail -c +$((abs+1)) \"$LOG\" | head -c $((dlen + 8)))\n");
    // Take the digits that follow the marker we located: a greedy match over the
    // whole window would read the *last* marker in it, so an interrupt landing next
    // to a completion reported the wrong status.
    out.push_str("tmp=${seg#\"$rpat\"}\n");
    out.push_str("code=${tmp%%[!0-9]*}\n");
    out.push_str("bel=$(printf '\\007'); after=${tmp#\"$code\"}\n");
    out.push_str("case \"$code\" in ''|*[!0-9]*) echo RHOST_ERR=protocol; exit 0;; esac\n");
    out.push_str("[ \"$code\" -le 255 ] 2>/dev/null || { echo RHOST_ERR=protocol; exit 0; }\n");
    out.push_str(
        "[ \"${after#\"$bel\"}\" != \"$after\" ] || { echo RHOST_ERR=protocol; exit 0; }\n",
    );
    out.push_str(&format!("echo \"RHOST_TOKEN={token}\"\n"));
    out.push_str("echo \"RHOST_EXIT=$code\"\n");
    out.push_str("printf 'RHOST_OUTPUT='\n");
    out.push_str("tail -c +$((start+1)) \"$LOG\" | head -c $((abs - start)) | base64 -w0\n");
    out.push_str("printf '\\n'\n");
    out
}
