//! `lifecycle`: list the sessions that exist, recover one from a runaway
//! program, and close one.

use super::shared::{
    FLOCK_PREFLIGHT, MARKER_SCAN_FUNC, RESOLVE_FUNC, SHELL_OF_FUNC, TMUX_PREFLIGHT, preamble,
};
use crate::session::DEFAULT_SHELL;
use crate::shell;
use std::time::Duration;

/// Prints one `RHOST_META` line per session: its id, the liveness of its tmux
/// session, and the base64 of its `meta.json`.
pub fn list() -> String {
    let mut out = preamble();
    out.push_str(TMUX_PREFLIGHT);
    out.push_str(
        r#"[ -d "$BASE/sessions" ] || exit 0
for d in "$BASE"/sessions/*/; do
  [ -f "$d/meta.json" ] || continue
  id=$(basename "$d")
  tmuxname=$(sed -n 's/.*"tmux_session"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$d/meta.json")
  alive=no
  if [ -n "$tmuxname" ] && tmux has-session -t "$tmuxname" 2>/dev/null; then alive=yes; fi
  meta=$(base64 -w0 < "$d/meta.json")
  printf 'RHOST_META\t%s\t%s\t%s\n' "$id" "$alive" "$meta"
done
"#,
    );
    out
}
/// Interrupts the pane under the same writer lock as exec and send, then proves
/// both that the managed shell owns the foreground and that its prompt emitted a
/// fresh marker. A REPL that catches Ctrl-C stays busy and is reported as such:
/// recover never types an exit command into a program it did not start.
pub fn recover(name_or_id: &str, timeout: Duration) -> String {
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
    out.push_str("DIR=\"$RHOST_DIR\"; LOG=\"${RHOST_DIR%/}/pty.log\"; LOCK=\"$DIR/lock\"; TMUX=\"$RHOST_TMUX\"\n");
    out.push_str("exec 9>\"$LOCK\" || { echo RHOST_ERR=nosession; exit 0; }\n");
    out.push_str("flock -n 9 || { echo RHOST_ERR=locked; exit 0; }\n");
    out.push_str(
        "tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n",
    );
    out.push_str(SHELL_OF_FUNC);
    out.push_str(
        "want=$(RHOST_SHELL_OF \"$DIR/meta.json\" 2>/dev/null); [ -n \"$want\" ] || want=",
    );
    out.push_str(&shell::quote(DEFAULT_SHELL));
    out.push('\n');
    out.push_str("TTY=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_tty}' 2>/dev/null)\n");
    out.push_str("[ -n \"$TTY\" ] || { echo RHOST_ERR=notty; exit 0; }\n");
    out.push_str("stty -echo < \"$TTY\" 2>/dev/null || { echo RHOST_ERR=notty; exit 0; }\n");
    out.push_str(MARKER_SCAN_FUNC);
    out.push_str("start=$(wc -c < \"$LOG\" 2>/dev/null || echo 0); dpat=$(printf '\\033]133;D;'); dlen=${#dpat}\n");
    out.push_str("tmux send-keys -t \"$TMUX:0.0\" C-c 2>/dev/null\n");
    out.push_str(&format!(
        "SECONDS=0; ready=0; floor=$((start + 1)); scan=$floor; while [ \"$SECONDS\" -lt {timeout_sec} ]; do\n"
    ));
    out.push_str(
        "  tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=sessiondied; exit 0; }\n",
    );
    out.push_str(
        "  fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n",
    );
    out.push_str("  if [ \"$fg\" = \"$want\" ] && rh_window; then ready=1; break; fi\n");
    out.push_str("  sleep 0.1\n");
    out.push_str("done\n");
    out.push_str(
        "fg=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_current_command}' 2>/dev/null)\n",
    );
    out.push_str("if [ -z \"$fg\" ]; then echo RHOST_ERR=unknownfg; exit 0; fi\n");
    out.push_str("if [ \"$fg\" != \"$want\" ]; then echo \"RHOST_FG=$fg\"; echo RHOST_ERR=busy; exit 0; fi\n");
    out.push_str("[ \"$ready\" = 1 ] || { echo RHOST_ERR=notready; exit 0; }\n");
    out.push_str("echo RHOST_OK=recovered\n");
    out
}
/// Kills the tmux session and removes its state directory.
pub fn close(name_or_id: &str) -> String {
    let mut out = String::new();
    out.push_str(&preamble());
    out.push_str(RESOLVE_FUNC);
    out.push_str(&format!(
        "RHOST_RESOLVE {} || {{ echo RHOST_ERR=nosession; exit 0; }}\n",
        shell::quote(name_or_id)
    ));
    out.push_str("tmux kill-session -t \"$RHOST_TMUX\" 2>/dev/null\n");
    out.push_str("rm -rf \"$RHOST_DIR\"\n");
    out.push_str("echo RHOST_OK=closed\n");
    out
}
