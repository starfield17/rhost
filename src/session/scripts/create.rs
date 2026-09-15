//! `create`: one tmux session, its metadata, and the pane integration.
//!
//! The integration is injected over the pane's stdin and never written to disk;
//! the readiness file it creates is what says every earlier line has run.

use super::shared::{FLOCK_PREFLIGHT, NAME_OF_FUNC, TMUX_PREFLIGHT, preamble};
use crate::base64;
use crate::session::Meta;
use crate::shell;
use crate::transport::protocol::DEFAULT_REMOTE_STATE_DIR;

/// Configures the pane's interactive bash. It is injected over the pane's stdin
/// and never written to disk. The final `: > ready` creates a readiness file, so
/// when it appears every earlier line has run (including `stty -echo`) and the
/// session is safe to use.
pub(super) fn integration_script(id: &str) -> String {
    let mut out = String::from(
        "__rh_done() { printf '\\033]133;D;%d\\007' \"$?\"; }\n\
         __rh_exec() { printf '\\033]133;C\\007'; }\n\
         trap '__rh_exec' DEBUG\n\
         PROMPT_COMMAND=__rh_done\n\
         PS1='' ; PS2=''\n\
         stty -echo 2>/dev/null\n\
         bind 'set enable-bracketed-paste off' 2>/dev/null\n\
         : > ",
    );
    // The path is expanded by the *pane's* shell, so it uses the same base
    // expression every script resolves.
    out.push_str(&format!(
        "\"${{RHOST_REMOTE_STATE:-{DEFAULT_REMOTE_STATE_DIR}}}/sessions/"
    ));
    out.push_str(id);
    out.push_str("/ready\"\n");
    out
}

/// Creates a session and blocks until its shell is ready.
pub fn create(meta: &Meta, pane_shell_command: &str) -> String {
    let mut out = String::new();
    out.push_str(&preamble());
    out.push_str(TMUX_PREFLIGHT);
    out.push_str(FLOCK_PREFLIGHT);
    out.push_str(NAME_OF_FUNC);
    if !meta.initial_cwd.is_empty() {
        if meta.initial_cwd == "~" {
            out.push_str("CWD=\"$HOME\"\n");
        } else {
            out.push_str(&format!("CWD={}\n", shell::path_quote(&meta.initial_cwd)));
        }
        // Resolve the requested directory in a subshell so the helper's own cwd
        // is untouched, and report the absolute path the account actually gets.
        // `~` never travels onward literally: the caller asked for a directory,
        // and which directory it turned out to be is a fact worth recording.
        out.push_str(
            "CWD=$(cd -- \"$CWD\" 2>/dev/null && printf '%s' \"$PWD\") || { echo RHOST_ERR=invalidcwd; exit 0; }\n",
        );
        out.push_str("[ -n \"$CWD\" ] || { echo RHOST_ERR=invalidcwd; exit 0; }\n");
        // A resolved path is written back as part of the session's remote state,
        // so `create` and a later `list` report the same directory. It is a
        // sidecar rather than a `meta.json` edit: the path is raw bytes and needs
        // no JSON escaping, and an old record without one falls back to the
        // metadata it was created with (no migration, no rewrite).
        out.push_str("echo \"RHOST_CWD=$CWD\"\n");
        out.push_str("RESOLVED_CWD=$CWD\n");
    }
    out.push_str("mkdir -p \"$BASE/sessions\" && chmod 700 \"$BASE/sessions\" 2>/dev/null || { echo RHOST_ERR=newfailed; exit 0; }\n");
    out.push_str("exec 8> \"$BASE/sessions/.create.lock\"\n");
    out.push_str("flock -w 30 8 || { echo RHOST_ERR=locked; exit 0; }\n");
    // Every tmux call closes the lock descriptor. tmux starts a server on its
    // first client, and a server that inherited this fd would hold the create
    // lock for as long as it lived — every later create would then wait out its
    // budget and report contention that does not exist. The pane inherits from
    // the server, never from us.
    out.push_str("RHOST_TMUX() { tmux \"$@\" 8>&-; }\n");
    // A duplicate name is refused before anything is created: resolving a name
    // takes the first match, so two sessions sharing one would behave ambiguously.
    out.push_str("for d in \"$BASE\"/sessions/*/; do\n  [ -f \"$d/meta.json\" ] || continue\n  [ \"$(RHOST_NAME_OF \"$d/meta.json\")\" = ");
    out.push_str(&shell::quote(&meta.name));
    out.push_str(" ] && { echo RHOST_ERR=nameinuse; exit 0; }\ndone\n");
    out.push_str(&format!("DIR=\"$BASE/sessions/{}\"\n", meta.id));
    out.push_str("LOG=\"$DIR/pty.log\"\n");
    out.push_str("mkdir -p \"$DIR\" && chmod 700 \"$DIR\" 2>/dev/null || { rm -rf \"$DIR\"; echo RHOST_ERR=newfailed; exit 0; }\n");
    out.push_str("rm -f \"$DIR/ready\" \"$LOG\"\n");

    let tmux = shell::quote(&meta.tmux_session);
    let pane = shell::quote(&format!("{}:0.0", meta.tmux_session));
    out.push_str("created=notyet; INTBUF=rhost_int_$$\n");
    out.push_str("cleanup() { RHOST_TMUX delete-buffer -b \"$INTBUF\" 2>/dev/null; if [ \"$created\" = yes ]; then RHOST_TMUX kill-session -t ");
    out.push_str(&tmux);
    out.push_str(" 2>/dev/null; rm -rf \"$DIR\"; fi; }\n");
    out.push_str("trap cleanup EXIT\n");
    out.push_str("trap 'exit 1' HUP INT TERM\n");
    out.push_str(&format!("RHOST_TMUX kill-session -t {tmux} 2>/dev/null\n"));
    if meta.initial_cwd.is_empty() {
        out.push_str(&format!(
            "RHOST_TMUX new-session -d -s {tmux} -x 220 -y 50 {} || {{ echo RHOST_ERR=newfailed; exit 0; }}\n",
            shell::quote(pane_shell_command)
        ));
    } else {
        out.push_str(&format!(
            "RHOST_TMUX new-session -d -s {tmux} -x 220 -y 50 -c \"$CWD\" {} || {{ echo RHOST_ERR=newfailed; exit 0; }}\n",
            shell::quote(pane_shell_command)
        ));
    }
    out.push_str("created=yes\n");
    out.push_str(&format!(
        "RHOST_TMUX set-option -t {tmux} history-limit 50000\n"
    ));
    // `pipe-pane` runs its command through `sh -c`, so the log path is quoted
    // with `%q` rather than pasted in.
    out.push_str("printf -v QPIPE 'cat >> %q' \"$LOG\"\n");
    out.push_str(&format!("RHOST_TMUX pipe-pane -t {pane} -o \"$QPIPE\"\n"));
    // tmux runs the pane command through the login shell, so an extra shell may
    // front bash: wait for the pane's foreground to become bash.
    out.push_str("i=0; while [ \"$i\" -lt 200 ]; do c=$(RHOST_TMUX display-message -p -t ");
    out.push_str(&pane);
    out.push_str(" '#{pane_current_command}' 2>/dev/null); [ \"$c\" = bash ] && break; sleep 0.1; i=$((i+1)); done\n");
    out.push_str(&format!(
        "printf '%s' '{}' | base64 -d | RHOST_TMUX load-buffer -b \"$INTBUF\" -\n",
        base64::encode(integration_script(&meta.id).as_bytes())
    ));
    out.push_str(&format!(
        "RHOST_TMUX paste-buffer -b \"$INTBUF\" -t {pane} 2>/dev/null\n"
    ));
    out.push_str("RHOST_TMUX delete-buffer -b \"$INTBUF\" 2>/dev/null\n");
    out.push_str(&format!("RHOST_TMUX send-keys -t {pane} Enter\n"));
    out.push_str("i=0; while [ \"$i\" -lt 100 ]; do [ -f \"$DIR/ready\" ] && break; sleep 0.1; i=$((i+1)); done\n");
    out.push_str(&format!(
        "if [ ! -f \"$DIR/ready\" ]; then RHOST_TMUX kill-session -t {tmux} 2>/dev/null; rm -rf \"$DIR\"; echo RHOST_ERR=notready; exit 0; fi\n"
    ));
    out.push_str("rm -f \"$DIR/ready\"\n");
    // Drop the bootstrap chatter so a read from offset 0 starts clean:
    // `pipe-pane`'s `cat` appends, so truncating is safe.
    out.push_str(": > \"$LOG\"\n");
    if !meta.initial_cwd.is_empty() {
        out.push_str(
            "printf '%s' \"$RESOLVED_CWD\" > \"$DIR/cwd\" || { echo RHOST_ERR=newfailed; exit 0; }\n",
        );
        out.push_str(
            "chmod 600 \"$DIR/cwd\" 2>/dev/null || { echo RHOST_ERR=newfailed; exit 0; }\n",
        );
    }
    let meta_json = serde_json::to_string(meta).unwrap_or_else(|_| "{}".to_string());
    out.push_str(&format!(
        "printf '%s' '{}' | base64 -d > \"$DIR/meta.json\" || {{ echo RHOST_ERR=newfailed; exit 0; }}\n",
        base64::encode(meta_json.as_bytes())
    ));
    out.push_str(
        "chmod 600 \"$DIR/meta.json\" 2>/dev/null || { echo RHOST_ERR=newfailed; exit 0; }\n",
    );
    // The commit point: the record is on disk and the cleanup trap is disarmed,
    // so the session will survive even if this helper dies now. `RHOST_CREATED`
    // is printed here, and only here — after this line the session is not the
    // trap's to remove, so the evidence a lost response leaves behind is honest.
    out.push_str("created=no\n");
    out.push_str("trap - EXIT HUP INT TERM\n");
    out.push_str(&format!("echo \"RHOST_CREATED={}\"\n", meta.id));
    out.push_str("echo RHOST_OK=created\n");
    out
}
