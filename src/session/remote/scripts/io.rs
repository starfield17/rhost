//! `io`: raw input into a pane, and the pane log read back out.

use super::shared::{FLOCK_PREFLIGHT, RESOLVE_FUNC, TMUX_PREFLIGHT, preamble};
use crate::base64;
use crate::shell;

/// What `send` injects: raw text, raw text followed by Enter, or one key name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SendKind {
    Data,
    DataEnter,
    Key,
}

/// Injects raw input into the pane. No completion boundary is expected: this is
/// the path for REPLs, debuggers and control keys.
pub fn send(name_or_id: &str, kind: SendKind, payload: &str) -> String {
    let mut out = String::new();
    out.push_str(&preamble());
    out.push_str(RESOLVE_FUNC);
    out.push_str(&format!(
        "RHOST_RESOLVE {} || {{ echo RHOST_ERR=nosession; exit 0; }}\n",
        shell::quote(name_or_id)
    ));
    out.push_str(TMUX_PREFLIGHT);
    out.push_str(FLOCK_PREFLIGHT);
    out.push_str("DIR=\"$RHOST_DIR\"; TMUX=\"$RHOST_TMUX\"; LOCK=\"$DIR/lock\"\n");
    out.push_str("exec 9>\"$LOCK\" || { echo RHOST_ERR=nosession; exit 0; }\n");
    out.push_str("flock -n 9 || { echo RHOST_ERR=locked; exit 0; }\n");
    out.push_str(
        "tmux has-session -t \"$TMUX\" 2>/dev/null || { echo RHOST_ERR=nosession; exit 0; }\n",
    );
    out.push_str("BUF=rhost_send_$$\n");
    out.push_str("cleanup() { tmux delete-buffer -b \"$BUF\" 2>/dev/null; }\n");
    out.push_str("trap cleanup EXIT; trap 'exit 1' HUP INT TERM\n");
    out.push_str("TTY=$(tmux display-message -p -t \"$TMUX:0.0\" '#{pane_tty}' 2>/dev/null)\n");
    out.push_str("[ -n \"$TTY\" ] || { echo RHOST_ERR=notty; exit 0; }\n");
    out.push_str("stty -echo < \"$TTY\" 2>/dev/null || { echo RHOST_ERR=notty; exit 0; }\n");
    if kind == SendKind::Key {
        out.push_str(&format!(
            "tmux send-keys -t \"$TMUX:0.0\" {}\n",
            shell::quote(payload)
        ));
    } else {
        out.push_str(&format!(
            "printf '%s' '{}' | base64 -d | tmux load-buffer -b \"$BUF\" -\n",
            base64::encode(payload.as_bytes())
        ));
        out.push_str("tmux paste-buffer -b \"$BUF\" -t \"$TMUX:0.0\" 2>/dev/null\n");
        out.push_str("tmux delete-buffer -b \"$BUF\" 2>/dev/null\n");
        if kind == SendKind::DataEnter {
            out.push_str("tmux send-keys -t \"$TMUX:0.0\" Enter\n");
        }
    }
    out.push_str("echo RHOST_OK=sent\n");
    out
}
/// Returns the raw pane log from byte offset `since`, base64-encoded, with
/// explicit cursors so a caller can poll incrementally.
pub fn read(name_or_id: &str, since: u64, max_bytes: usize) -> String {
    let max_bytes = if max_bytes == 0 {
        256 * 1024
    } else {
        max_bytes
    };
    let mut out = String::new();
    out.push_str(&preamble());
    out.push_str(RESOLVE_FUNC);
    out.push_str(&format!(
        "RHOST_RESOLVE {} || {{ echo RHOST_ERR=nosession; exit 0; }}\n",
        shell::quote(name_or_id)
    ));
    out.push_str("echo \"RHOST_ID=$RHOST_ID\"\n");
    out.push_str("DIR=\"$RHOST_DIR\"; LOG=\"$DIR/pty.log\"\n");
    out.push_str("LOG=$(printf '%s' \"$LOG\" | sed 's#/$##')\n");
    out.push_str("size=$(wc -c < \"$LOG\" 2>/dev/null || echo 0)\n");
    out.push_str(&format!("since={since}\n"));
    out.push_str("[ \"$since\" -gt \"$size\" ] && since=$size\n");
    out.push_str(&format!(
        "inc=$(tail -c +$((since+1)) \"$LOG\" 2>/dev/null | head -c {max_bytes} | wc -c)\n"
    ));
    out.push_str("echo \"RHOST_FROM=$since\"\n");
    out.push_str("echo \"RHOST_NEXT=$((since + inc))\"\n");
    out.push_str("echo \"RHOST_SIZE=$size\"\n");
    out.push_str("tail -c +$((since+1)) \"$LOG\" 2>/dev/null | head -c \"$inc\" | base64 -w0\n");
    out.push_str("echo\n");
    out
}
