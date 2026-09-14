use super::support::{Live, boolean, number, text, unique, write};

#[test]
fn smoke_core_workflows_cross_process_boundaries() -> Result<(), String> {
    let live = Live::new("smoke")?;
    let host = live.host().to_string();
    let doctor = live.ok(&["--json", "doctor", &host, "--timeout", "0"])?;
    assert_eq!(doctor.value["operation"], "doctor");
    assert!(boolean(&doctor.value, "/data/online")?);
    assert!(boolean(&doctor.value, "/data/state_dir_writable")?);

    let exec = live.exec("printf smoke-exec; printf smoke-error >&2")?;
    assert_eq!(number(&exec.value, "/data/execution/exit_code")?, 0);
    assert_eq!(
        text(&exec.value, "/data/output/stdout/content")?,
        "smoke-exec"
    );
    assert_eq!(
        text(&exec.value, "/data/output/stderr/content")?,
        "smoke-error"
    );

    let remote = live.remote_dir()?;
    let local = live.path("smoke.txt");
    write(&local, b"smoke-file\n")?;
    let path = format!("{remote}/nested/file.txt");
    let written = live.ok(&[
        "--json",
        "fs",
        "write",
        &host,
        &path,
        "--from",
        &local.to_string_lossy(),
        "--parents",
    ])?;
    let read = live.ok(&["--json", "fs", "read", &host, &path])?;
    assert_eq!(text(&read.value, "/data/content")?, "smoke-file\n");
    assert_eq!(
        read.value["data"]["sha256"],
        written.value["data"]["sha256"]
    );

    let name = unique("rlive-smoke-session");
    let created = live.ok(&[
        "--json", "session", "create", &host, "--name", &name, "--cwd", "/tmp",
    ])?;
    let id = text(&created.value, "/data/session_id")?.to_string();
    let result: Result<(), String> = (|| {
        live.ok(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "export RHOST_SMOKE_VALUE=42; cd /var/log",
        ])?;
        let later = live.ok(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "printf '%s|%s' \"$PWD\" \"$RHOST_SMOKE_VALUE\"",
        ])?;
        assert_eq!(text(&later.value, "/data/session_id")?, id);
        assert!(text(&later.value, "/data/output/content")?.contains("/var/log|42"));
        Ok(())
    })();
    let closed = live.ok(&["--json", "session", "close", &host, &name]);
    live.cleanup_remote(&remote)?;
    result?;
    closed?;
    Ok(())
}
