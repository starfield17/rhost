use std::process::Stdio;
use std::time::Duration;

use super::support::{Live, decode, number, poll, shell_quote, text, unique, wait_bounded};

const HOLD_SECONDS: u64 = 12;

#[test]
fn exact_program_streams_cwd_and_remote_status_are_preserved() -> Result<(), String> {
    let live = Live::new("exec-basic")?;
    let host = live.host().to_string();
    let mut command = live.command(&[
        "exec",
        &host,
        "--cwd",
        "/var/log",
        "--command",
        "cat; pwd >&2; exit 4",
    ]);
    command.stdin(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn exec: {error}"))?;
    use std::io::Write;
    child
        .stdin
        .take()
        .ok_or("exec stdin was not piped")?
        .write_all(b"streamed input\n")
        .map_err(|error| format!("write exec stdin: {error}"))?;
    let output = wait_bounded(child, Duration::from_secs(90))?;
    assert_eq!(output.status.code(), Some(4));
    assert_eq!(output.stdout, b"streamed input\n");
    assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "/var/log");

    let json = live.ok(&[
        "--json",
        "exec",
        &host,
        "--command",
        "printf out; printf err >&2; exit 7",
    ])?;
    assert_eq!(json.output.status.code(), Some(7));
    assert_eq!(number(&json.value, "/data/execution/exit_code")?, 7);
    assert_eq!(text(&json.value, "/data/output/stdout/content")?, "out");
    assert_eq!(text(&json.value, "/data/output/stderr/content")?, "err");
    Ok(())
}

#[test]
fn completed_foreground_survives_an_inherited_background_pipe() -> Result<(), String> {
    let live = Live::new("exec-pipe")?;
    let host = live.host().to_string();
    let command = format!("sh -c \"echo done; sleep {HOLD_SECONDS} & exit 0\"");
    let answer = live.ok(&["--json", "exec", &host, "--command", &command])?;
    assert_eq!(answer.output.status.code(), Some(0));
    assert_eq!(answer.value["data"]["execution"]["status"], "completed");
    assert_eq!(number(&answer.value, "/data/execution/exit_code")?, 0);
    assert_eq!(
        text(&answer.value, "/data/output/stdout/content")?,
        "done\n"
    );
    assert_eq!(answer.value["data"]["cleanup"]["status"], "not_attempted");
    Ok(())
}

#[test]
fn deadline_after_completion_keeps_the_foreground_exit_code() -> Result<(), String> {
    let live = Live::new("exec-after-completion")?;
    let host = live.host().to_string();
    let command = format!("sh -c \"echo done; sleep {HOLD_SECONDS} & exit 0\"");
    let answer = live.error(
        "REMOTE_COMMAND_TIMEOUT",
        &[
            "--timeout",
            "6s",
            "--json",
            "exec",
            &host,
            "--command",
            &command,
        ],
    )?;
    assert_eq!(answer.output.status.code(), Some(124));
    assert_eq!(answer.value["data"]["execution"]["status"], "completed");
    assert_eq!(number(&answer.value, "/data/execution/exit_code")?, 0);
    assert_eq!(answer.value["data"]["cleanup"]["status"], "not_attempted");
    assert_eq!(answer.value["error"]["retryable"], false);
    Ok(())
}

#[test]
fn timeout_and_sigint_stop_the_owned_remote_process_group() -> Result<(), String> {
    let live = Live::new("exec-stop")?;
    let host = live.host().to_string();
    let remote = live.remote_dir()?;
    let marker = format!("{remote}/started");
    let token = unique("rlive-owned-process");
    let program = format!(
        "printf ready > {}; exec -a {} sleep 300",
        shell_quote(&marker),
        shell_quote(&token)
    );
    let child = live
        .command(&["--json", "exec", &host, "--command", &program])
        .spawn()
        .map_err(|error| format!("spawn cancellable exec: {error}"))?;
    poll(|| {
        let probe = live.exec(&format!("test -e {}", shell_quote(&marker)))?;
        Ok(number(&probe.value, "/data/execution/exit_code")? == 0)
    })?;
    let status = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .map_err(|error| format!("signal rhost: {error}"))?;
    assert!(status.success());
    let output = wait_bounded(child, Duration::from_secs(90))?;
    let answer = decode(output, &["cancelled exec"])?;
    assert_eq!(answer.output.status.code(), Some(130));
    assert_eq!(answer.value["error"]["code"], "REMOTE_COMMAND_CANCELLED");
    assert_eq!(
        answer.value["data"]["cleanup"]["status"],
        "confirmed_stopped"
    );
    poll(|| {
        let probe = live.exec(&format!(
            "ps -eo args | grep -F {} | grep -v grep",
            shell_quote(&token)
        ))?;
        Ok(number(&probe.value, "/data/execution/exit_code")? != 0)
    })?;
    live.cleanup_remote(&remote)
}

#[test]
fn bounded_capture_reports_source_bytes() -> Result<(), String> {
    let live = Live::new("exec-capture")?;
    let host = live.host().to_string();
    let answer = live.ok(&[
        "--json",
        "exec",
        &host,
        "--max-output-bytes",
        "1000",
        "--command",
        "head -c 100000 /dev/zero | tr '\\0' x",
    ])?;
    assert_eq!(
        text(&answer.value, "/data/output/stdout/content")?.len(),
        1000
    );
    assert_eq!(number(&answer.value, "/data/output/stdout/bytes")?, 100000);
    assert_eq!(answer.value["data"]["output"]["stdout"]["truncated"], true);
    Ok(())
}
