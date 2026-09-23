//! Exercise the generated session helper in bash with failing tmux commands.
//! This uses local stand-ins for tools; no SSH target or real pane is needed.

use rhost::session::remote::scripts::{SendKind, close, send};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn executable(path: &Path, body: &str) -> Result<(), String> {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).map_err(|error| error.to_string())?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())
}

fn run(script: &str, root: &Path, mode: &str) -> Result<String, String> {
    let output = Command::new("/bin/bash")
        .arg("-c")
        .arg(script)
        .env("RHOST_REMOTE_STATE", root.join("remote"))
        .env("RHOST_TEST_MODE", mode)
        .env("RHOST_TEST_TTY", root.join("tty"))
        .env("RHOST_TEST_KILLED", root.join("killed"))
        .env(
            "PATH",
            format!("{}:/usr/bin:/bin", root.join("bin").display()),
        )
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "helper exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

#[test]
fn send_and_close_refuse_unconfirmed_tmux_effects() -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("rhost-session-helper-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let bin = root.join("bin");
    let session = root.join("remote/sessions/s_ab");
    std::fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&session).map_err(|error| error.to_string())?;
    std::fs::write(root.join("tty"), b"").map_err(|error| error.to_string())?;
    std::fs::write(
        session.join("meta.json"),
        r#"{"schema_version":1,"id":"s_ab","name":"work","tmux_session":"rhost_s_ab","created_at":"now"}"#,
    )
    .map_err(|error| error.to_string())?;
    executable(
        &bin.join("tmux"),
        r#"case "$1" in
  has-session) [ -f "$RHOST_TEST_KILLED" ] && exit 1; exit 0 ;;
  display-message) printf '%s\n' "$RHOST_TEST_TTY"; exit 0 ;;
  load-buffer) [ "$RHOST_TEST_MODE" = loadfail ] && exit 1; cat >/dev/null; exit 0 ;;
  paste-buffer) [ "$RHOST_TEST_MODE" = pastefail ] && exit 1; exit 0 ;;
  send-keys) case "$RHOST_TEST_MODE" in keyfail|enterfail) exit 1;; esac; exit 0 ;;
  delete-buffer) exit 0 ;;
  kill-session) [ "$RHOST_TEST_MODE" = killfail ] && exit 1; : > "$RHOST_TEST_KILLED"; exit 0 ;;
esac
exit 2"#,
    )?;
    executable(&bin.join("flock"), "exit 0")?;
    executable(&bin.join("stty"), "exit 0")?;
    executable(
        &bin.join("rm"),
        "[ \"$RHOST_TEST_MODE\" = rmfail ] && exit 1\nexec /bin/rm \"$@\"",
    )?;

    for (mode, kind, wanted) in [
        ("loadfail", SendKind::Data, "RHOST_ERR=inputfailed"),
        ("pastefail", SendKind::Data, "RHOST_ERR=inputuncertain"),
        ("enterfail", SendKind::DataEnter, "RHOST_ERR=inputuncertain"),
        ("keyfail", SendKind::Key, "RHOST_ERR=inputuncertain"),
    ] {
        let stdout = run(&send("work", kind, "text"), &root, mode)?;
        assert!(
            stdout.lines().any(|line| line == wanted),
            "{mode}: {stdout}"
        );
        assert!(!stdout.contains("RHOST_OK=sent"), "{mode}: {stdout}");
    }
    let stdout = run(&close("work"), &root, "killfail")?;
    assert!(stdout.contains("RHOST_ERR=closefailed"), "{stdout}");
    assert!(
        session.join("meta.json").exists(),
        "failed kill keeps state"
    );
    let stdout = run(&close("work"), &root, "rmfail")?;
    assert!(stdout.contains("RHOST_ERR=closefailed"), "{stdout}");
    assert!(
        session.join("meta.json").exists(),
        "failed cleanup keeps its record"
    );
    let stdout = run(&close("work"), &root, "present")?;
    assert!(stdout.contains("RHOST_OK=closed"), "{stdout}");
    assert!(!session.exists(), "confirmed close removes state");
    std::fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(())
}
