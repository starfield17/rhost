use std::time::Duration;

use super::support::{Live, number, poll, text, unique, wait_bounded};

#[test]
fn session_state_identity_and_byte_cursor_survive_cli_processes() -> Result<(), String> {
    let live = Live::new("session-state")?;
    let host = live.host().to_string();
    let name = unique("rlive-session");
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
            "cd /var/log; export RHOST_LIVE_VALUE=retained",
        ])?;
        let one = live.ok(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "printf '%s|%s|%s' \"$PWD\" \"$RHOST_LIVE_VALUE\" \"$$\"",
        ])?;
        let two = live.ok(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "printf '%s' \"$$\"",
        ])?;
        assert_eq!(text(&one.value, "/data/session_id")?, id);
        assert_eq!(text(&one.value, "/data/session_ref")?, name);
        let first = text(&one.value, "/data/output/content")?;
        let shell_pid = first.rsplit('|').next().ok_or("missing shell pid")?.trim();
        assert!(first.contains("/var/log|retained|"));
        assert!(text(&two.value, "/data/output/content")?.contains(shell_pid));

        live.ok(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "printf 'cursor-中文'",
        ])?;
        let read = live.ok(&["--json", "session", "read", &host, &name, "--since", "0"])?;
        let next = number(&read.value, "/data/next")?;
        assert!(next > 0);
        let later = live.ok(&[
            "--json",
            "session",
            "read",
            &host,
            &name,
            "--since",
            &next.to_string(),
        ])?;
        assert!(!text(&later.value, "/data/content")?.contains("cursor-中文"));
        Ok(())
    })();
    let closed = live.ok(&["--json", "session", "close", &host, &name]);
    result?;
    closed?;
    Ok(())
}

#[test]
fn busy_program_refuses_exec_and_recover_restores_the_shell() -> Result<(), String> {
    let live = Live::new("session-busy")?;
    let host = live.host().to_string();
    let name = unique("rlive-busy");
    live.ok(&["--json", "session", "create", &host, "--name", &name])?;
    let result: Result<(), String> = (|| {
        live.ok(&[
            "--json", "session", "send", &host, &name, "--data", "cat", "--enter",
        ])?;
        poll(|| {
            match live.error(
                "SESSION_BUSY",
                &[
                    "--json",
                    "session",
                    "exec",
                    &host,
                    &name,
                    "--command",
                    "echo must-not-run",
                ],
            ) {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        })?;
        let read = live.ok(&["--json", "session", "read", &host, &name, "--since", "0"])?;
        assert!(!text(&read.value, "/data/content")?.contains("must-not-run"));
        let recovered = live.ok(&["--json", "session", "recover", &host, &name])?;
        assert_eq!(recovered.value["data"]["session_preserved"], true);
        let after = live.ok(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "printf recovered",
        ])?;
        assert!(text(&after.value, "/data/output/content")?.contains("recovered"));
        Ok(())
    })();
    let closed = live.ok(&["--json", "session", "close", &host, &name]);
    result?;
    closed?;
    Ok(())
}

#[test]
fn killing_one_cli_keeps_the_remote_writer_and_session_alive() -> Result<(), String> {
    let live = Live::new("session-client-death")?;
    let host = live.host().to_string();
    let name = unique("rlive-death");
    live.ok(&["--json", "session", "create", &host, "--name", &name])?;
    let mut child = live
        .command(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "sleep 8; echo finished",
        ])
        .spawn()
        .map_err(|error| format!("spawn session writer: {error}"))?;
    std::thread::sleep(Duration::from_secs(3));
    child
        .kill()
        .map_err(|error| format!("kill local CLI: {error}"))?;
    let _ = wait_bounded(child, Duration::from_secs(10));
    let busy = live.error(
        "SESSION_UNHEALTHY",
        &[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "echo refused",
        ],
    )?;
    assert_eq!(busy.value["error"]["retryable"], true);
    poll(|| {
        Ok(live
            .ok(&[
                "--json",
                "session",
                "exec",
                &host,
                &name,
                "--command",
                "true",
            ])
            .is_ok())
    })?;
    live.ok(&["--json", "session", "close", &host, &name])?;
    Ok(())
}

#[test]
fn raw_send_exit_status_unknown_target_and_recovery_are_explicit() -> Result<(), String> {
    let live = Live::new("session-protocol")?;
    let host = live.host().to_string();
    let missing = unique("rlive-missing");
    let absent = live.error(
        "SESSION_NOT_FOUND",
        &[
            "--json",
            "session",
            "exec",
            &host,
            &missing,
            "--command",
            "true",
        ],
    )?;
    assert_eq!(absent.output.status.code(), Some(255));

    let name = unique("rlive-protocol");
    live.ok(&["--json", "session", "create", &host, "--name", &name])?;
    let result: Result<(), String> = (|| {
        for code in [0, 4, 7, 11] {
            let command = format!("sh -c 'exit {code}'");
            let answer = live.ok(&[
                "--json",
                "session",
                "exec",
                &host,
                &name,
                "--command",
                &command,
            ])?;
            assert_eq!(number(&answer.value, "/data/execution/exit_code")?, code);
            assert_eq!(answer.output.status.code(), Some(code as i32));
        }
        live.ok(&[
            "--json",
            "session",
            "send",
            &host,
            &name,
            "--data",
            "printf raw-send-ok",
            "--enter",
        ])?;
        poll(|| {
            let answer = live.ok(&["--json", "session", "read", &host, &name, "--since", "0"])?;
            Ok(text(&answer.value, "/data/content")?.contains("raw-send-ok"))
        })?;
        live.ok(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "export RHOST_RECOVERED_VALUE=retained",
        ])?;
        let recovered = live.ok(&["--json", "session", "recover", &host, &name])?;
        assert_eq!(recovered.value["data"]["session_preserved"], true);
        let retained = live.ok(&[
            "--json",
            "session",
            "exec",
            &host,
            &name,
            "--command",
            "printf '%s' \"$RHOST_RECOVERED_VALUE\"",
        ])?;
        assert!(text(&retained.value, "/data/output/content")?.contains("retained"));
        Ok(())
    })();
    let closed = live.ok(&["--json", "session", "close", &host, &name]);
    result?;
    closed?;
    Ok(())
}
