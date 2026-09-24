//! The remote helper programs, one per session operation.
//!
//! Each script is a complete shell program: it runs under `bash -lc` through the
//! ordinary exec wrapper, so the wrapper's markers stay out of the helper's own
//! stdout and the helper reports its outcome as `RHOST_*` lines that
//! [`super::protocol`] parses.
//!
//! Three properties of these scripts are load-bearing and are why they are
//! written out rather than assembled from pieces:
//!
//!   * **ownership** — every writer takes the session's `flock` target with
//!     `flock -n`, so a second writer is refused instead of interleaving;
//!   * **boundaries** — the pane's bash emits one OSC 133 marker per command,
//!     carrying the exit status, and exec's own marker additionally carries the
//!     invocation token, so completion is evidence rather than a guess;
//!   * **fail-closed** — anything that types into the pane first checks that the
//!     managed shell owns the foreground, and refuses with the name of what does
//!     instead of pasting into a REPL.
//!
//! The programs are split by what they do to a session: `create` makes one,
//! `exec` runs one command in it, `io` injects input and reads the pane log
//! back, and `lifecycle` lists, recovers and closes. `shared` holds what every
//! program starts with.

mod create;
mod exec;
mod io;
mod lifecycle;
mod shared;

pub use create::create;
pub use exec::exec;
pub use io::{SendKind, read, send};
pub use lifecycle::{close, list, recover};

#[cfg(test)]
mod behavior_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base64;
    use crate::session::{DEFAULT_SHELL, Meta};
    use crate::transport::DEFAULT_REMOTE_STATE_DIR;
    use std::time::Duration;

    use super::create::integration_script;

    fn meta() -> Meta {
        Meta::new("s_ab12", "work", "/tmp", DEFAULT_SHELL, "now")
    }

    #[test]
    fn every_writer_takes_the_lock_without_waiting_and_resolves_by_name() {
        for script in [
            exec("work", "true", Duration::from_secs(60), &"a".repeat(32)),
            send("work", SendKind::Data, "x"),
            recover("work", Duration::from_secs(30)),
            close("work"),
        ] {
            assert!(
                script.contains("flock -n 9 || { echo RHOST_ERR=locked; exit 0; }"),
                "{script}"
            );
            assert!(
                script.contains("RHOST_RESOLVE 'work' || { echo RHOST_ERR=nosession; exit 0; }"),
                "{script}"
            );
            assert!(script.contains("command -v tmux >/dev/null 2>&1"));
            assert!(script.contains("command -v flock >/dev/null 2>&1"));
        }
    }

    #[test]
    fn send_and_close_only_report_success_after_their_effects_are_checked() {
        for script in [
            send("work", SendKind::Data, "text"),
            send("work", SendKind::DataEnter, "text"),
            send("work", SendKind::Key, "C-c"),
        ] {
            assert!(script.contains("RHOST_ERR=inputuncertain"), "{script}");
            assert!(script.find("RHOST_ERR=inputuncertain") < script.find("RHOST_OK=sent"));
        }
        assert!(send("work", SendKind::Data, "text").contains("RHOST_ERR=inputfailed"));
        let script = close("work");
        assert!(script.contains("RHOST_ERR=closefailed"), "{script}");
        assert!(script.contains("flock -n 9"), "{script}");
        assert!(script.find("RHOST_ERR=closefailed") < script.find("RHOST_OK=closed"));
    }

    #[test]
    fn exec_pastes_one_compound_command_with_the_token_in_its_marker() {
        let token = "f".repeat(32);
        let script = exec("work", "printf hi", Duration::from_secs(5), &token);
        let payload =
            format!("{{\nprintf hi\ncommand printf '\\033]133;R;{token};%d\\007' \"$?\"\n}}");
        assert!(script.contains(&format!(
            "cmd=$(printf '%s' '{}' | base64 -d)",
            base64::encode(payload.as_bytes())
        )));
        // The newline that submits the command rides inside the same paste.
        assert!(script.contains(
            "printf '%s\\n' \"$cmd\" | tmux load-buffer -b \"$BUF\" - \\; paste-buffer -d"
        ));
        assert!(script.contains(&format!("rpat=$(printf '\\033]133;R;{token};')")));
        // A second foreground check runs *after* the idle wait, because the pane
        // can change hands while a command is being submitted.
        assert_eq!(script.matches("echo RHOST_ERR=busy").count(), 2);
        assert!(script.contains("RHOST_ERR=inputfailed"));
        assert!(script.contains("RHOST_RECOVERED=$recovered"));
        assert!(script.contains("while [ \"$SECONDS\" -lt 5 ]"));
    }

    #[test]
    fn create_waits_for_readiness_and_refuses_a_duplicate_name() {
        let script = create(&meta(), "bash --noprofile --norc -i");
        let integration = integration_script("s_ab12");
        // The pane's integration is injected over stdin, never written to disk;
        // its readiness file is what says every earlier line ran.
        assert!(
            integration.contains(&format!(
                ": > \"${{RHOST_REMOTE_STATE:-{DEFAULT_REMOTE_STATE_DIR}}}/sessions/s_ab12/ready\""
            )),
            "{integration}"
        );
        assert!(integration.contains("stty -echo"));
        assert!(integration.contains("PROMPT_COMMAND=__rh_done"));
        assert!(script.contains(&base64::encode(integration.as_bytes())));
        assert!(script.contains("echo RHOST_ERR=nameinuse; exit 0;"));
        assert!(script.contains("echo RHOST_ERR=notready; exit 0;"));
        assert!(script.contains("RHOST_TMUX pipe-pane -t 'rhost_s_s_ab12:0.0' -o \"$QPIPE\""));
        assert!(script.contains("base64 -d > \"$DIR/meta.json\""));
        assert!(script.contains("CWD='/tmp'"));
        // A tmux server started while the create lock is held would inherit that
        // fd and hold the lock for the rest of its life, so every call closes it.
        assert!(script.contains("RHOST_TMUX() { tmux \"$@\" 8>&-; }\n"));
        for line in script.lines() {
            assert!(
                !line.trim_start().starts_with("tmux "),
                "this tmux call keeps the create lock open: {line}"
            );
        }
        assert!(script.ends_with("echo RHOST_OK=created\n"));
        // Creation evidence is printed only after the cleanup trap is disarmed,
        // so a helper the transport killed can never leave behind evidence for a
        // session its own EXIT trap then removed.
        let trap_off = script
            .find("trap - EXIT HUP INT TERM")
            .unwrap_or(usize::MAX);
        let created_evidence = script.find("RHOST_CREATED=").unwrap_or(usize::MAX);
        assert!(
            trap_off < created_evidence,
            "RHOST_CREATED must follow the commit point, not precede it: {script}"
        );
    }

    #[test]
    fn read_reports_cursors_and_bounds_the_page() {
        let script = read("work", 7, 0);
        assert!(script.contains("since=7\n"));
        assert!(script.contains("RHOST_FROM=$since"));
        assert!(script.contains("RHOST_NEXT=$((since + inc))"));
        assert!(script.contains("RHOST_SIZE=$size"));
        assert!(script.contains("head -c 262144"));
        assert!(read("work", 0, 16).contains("head -c 16"));
    }

    #[test]
    fn send_supports_verbatim_text_one_key_and_text_then_enter() {
        let data = send("work", SendKind::Data, "a'b");
        assert!(data.contains(&base64::encode(b"a'b")));
        assert!(!data.contains("send-keys -t \"$TMUX:0.0\" Enter"));
        assert!(
            send("work", SendKind::DataEnter, "ls")
                .contains("tmux send-keys -t \"$TMUX:0.0\" Enter")
        );
        assert!(
            send("work", SendKind::Key, "C-c").contains("tmux send-keys -t \"$TMUX:0.0\" 'C-c'")
        );
        assert!(data.ends_with("echo RHOST_OK=sent\n"));
    }

    #[test]
    fn close_kills_the_tmux_session_and_removes_the_record() {
        let script = close("s_ab12");
        assert!(script.contains("tmux kill-session -t \"$RHOST_TMUX\" 2>/dev/null"));
        assert!(script.contains("rm -rf \"$RHOST_DIR\""));
        assert!(script.ends_with("echo RHOST_OK=closed\n"));
    }
}
