use std::time::Duration;

use super::support::{Live, boolean, number, poll, text, wait_bounded};

#[test]
fn doctor_and_connection_reuse_are_real_across_processes() -> Result<(), String> {
    let live = Live::new("transport-reuse")?;
    let host = live.host().to_string();
    let first = live.ok(&["--json", "doctor", &host, "--timeout", "0"])?;
    assert!(boolean(&first.value, "/data/online")?);
    for capability in ["bash", "tmux", "flock", "python3", "rsync"] {
        assert_eq!(
            first.value["data"]["capabilities"][capability], true,
            "{capability}"
        );
    }
    live.exec("true")?;
    let one = live.ok(&["--json", "connection", "status", &host])?;
    assert_eq!(text(&one.value, "/data/master_status")?, "alive");
    let pid = number(&one.value, "/data/master_pid")?;
    live.exec("true")?;
    let two = live.ok(&["--json", "connection", "status", &host])?;
    assert_eq!(number(&two.value, "/data/master_pid")?, pid);
    Ok(())
}

/// The capability paths `doctor` reports must be the paths the *same* execution
/// environment resolves: a real run's `command -v` for the same names.
#[test]
fn doctor_capability_paths_match_a_real_command_v() -> Result<(), String> {
    let live = Live::new("transport-paths")?;
    let host = live.host().to_string();
    let doctor = live.ok(&["--json", "doctor", &host, "--timeout", "0"])?;
    assert!(boolean(&doctor.value, "/data/online")?);

    for capability in ["bash", "tmux", "flock", "rsync", "python3"] {
        assert_eq!(
            doctor.value["data"]["capabilities"][capability], true,
            "{capability} should be present on the test host"
        );
        let reported = text(
            &doctor.value,
            &format!("/data/capability_paths/{capability}"),
        )?
        .to_string();
        assert!(
            reported.starts_with('/'),
            "{capability} path must be absolute: {reported:?}"
        );
        // Ask the host directly, the way a run would resolve the name.
        let probe = live.exec(&format!("command -v {capability}"))?;
        let observed = text(&probe.value, "/data/output/stdout/content")?
            .trim()
            .to_string();
        assert_eq!(
            reported, observed,
            "doctor must report what command -v resolves for {capability}"
        );
    }
    Ok(())
}

#[test]
fn reset_does_not_kill_an_already_accepted_channel() -> Result<(), String> {
    let live = Live::new("transport-reset")?;
    let host = live.host().to_string();
    let remote = live.remote_dir()?;
    let marker = format!("{remote}/accepted");
    let command = format!("printf yes > '{marker}'; sleep 3; printf survived");
    let child = live
        .command(&["--json", "exec", &host, "--command", &command])
        .spawn()
        .map_err(|error| format!("spawn accepted channel: {error}"))?;
    poll(|| {
        let answer = live.exec(&format!("test -e '{marker}'"))?;
        Ok(number(&answer.value, "/data/execution/exit_code")? == 0)
    })?;
    let reset = live.ok(&["--json", "connection", "reset", &host])?;
    assert!(boolean(&reset.value, "/data/stopped")?);
    let output = wait_bounded(child, Duration::from_secs(30))?;
    let answer = super::support::decode(output, &["accepted channel"])?;
    assert_eq!(
        text(&answer.value, "/data/output/stdout/content")?,
        "survived"
    );
    live.cleanup_remote(&remote)
}

#[test]
fn deep_cache_path_falls_back_without_losing_reuse() -> Result<(), String> {
    let live = Live::new("transport-deep-cache")?;
    let host = live.host().to_string();
    let deep = live
        .path("deep-dir-deep-dir-deep-dir-deep-dir-deep-dir-deep-dir-deep-dir-deep-dir")
        .join("Library/Caches/rhost");
    std::fs::create_dir_all(&deep).map_err(|error| format!("create deep cache: {error}"))?;
    let args = ["--json", "exec", &host, "--command", "printf deep-cache-ok"];
    let first = live.json_with_env(&args, "RHOST_CACHE_DIR", &deep)?;
    assert_eq!(first.value["ok"], true, "{}", first.value);
    let status = live.json_with_env(
        &["--json", "connection", "status", &host],
        "RHOST_CACHE_DIR",
        &deep,
    )?;
    assert_eq!(text(&status.value, "/data/master_status")?, "alive");
    Ok(())
}
