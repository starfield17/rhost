//! Moving files: `scp` for one file, `rsync` for a tree.

use crate::hosts::{local_host, rsync_stub, scp_stub};
use crate::support::{Harness, envelope, fail, run, want_code};

#[test]
fn a_put_builds_one_scp_invocation_and_reports_the_landing_file() -> Result<(), String> {
    let harness = Harness::open("fs-put")?;
    local_host(&harness)?;
    harness.stub("scp", "exit 0")?;
    let local = harness.path("model.py");
    std::fs::write(&local, "print('hello')\n").map_err(|e| fail("write fixture", e))?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "put",
            "gpu",
            &local.to_string_lossy(),
            "/srv/app/model.py",
            "--json",
        ],
    )?;
    if outcome.status != 0 {
        return Err(format!("{value}\ncalls:\n{}", harness.calls_text()));
    }
    assert_eq!(value["operation"], "fs.put");
    assert_eq!(value["data"]["backend"], "scp");
    assert_eq!(value["data"]["size"], 15);
    assert_eq!(value["data"]["destination"], "gpu:/srv/app/model.py");
    assert_eq!(value["data"]["checksum_verified"], false);
    let calls = harness.tool_calls("scp");
    assert_eq!(
        calls.len(),
        1,
        "one scp invocation, not a retry loop; calls were:\n{}",
        harness.calls_text()
    );
    let arguments = &calls[0].args;
    assert_eq!(arguments[arguments.len() - 2], local.to_string_lossy());
    assert_eq!(arguments[arguments.len() - 1], "gpu:/srv/app/model.py");
    assert!(arguments.iter().any(|a| a == "-q"));
    assert!(
        arguments.iter().any(|a| a.starts_with("ControlPath=")),
        "the copy rides the same master as every other command: {arguments:?}"
    );
    Ok(())
}

#[test]
fn a_local_colon_filename_is_prefixed_so_scp_cannot_read_it_as_a_host() -> Result<(), String> {
    let harness = Harness::open("fs-colon")?;
    local_host(&harness)?;
    harness.stub("scp", "exit 0")?;
    std::fs::write(harness.path("a:b.txt"), "colon\n").map_err(|e| fail("fixture", e))?;
    let outcome = run(&harness, &["fs", "put", "gpu", "a:b.txt", "/tmp/a:b.txt"])?;
    assert_eq!(outcome.status, 0, "{}", outcome.stderr);
    let calls = harness.tool_calls("scp");
    assert_eq!(calls[0].args[calls[0].args.len() - 2], "./a:b.txt");
    Ok(())
}

#[test]
fn a_get_into_an_existing_directory_names_the_file_it_created() -> Result<(), String> {
    let harness = Harness::open("fs-get-dir")?;
    harness.stub("scp", &scp_stub("printf 'fetched' > \"$dest\""))?;
    let download = harness.path("download");
    std::fs::create_dir_all(&download).map_err(|e| fail("fixture", e))?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "get",
            "gpu",
            "/srv/app/model.py",
            &download.to_string_lossy(),
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 0, "{value}");
    let reported = value["data"]["destination"].as_str().unwrap_or("");
    assert_eq!(reported, download.join("model.py").to_string_lossy());
    assert_eq!(value["data"]["size"], 7);
    let body = std::fs::read(download.join("model.py")).map_err(|e| fail("read back", e))?;
    assert_eq!(body, b"fetched");
    Ok(())
}

#[test]
fn scp_success_without_a_landing_file_is_a_transfer_failure() -> Result<(), String> {
    let harness = Harness::open("fs-get-no-file")?;
    harness.stub("scp", "exit 0")?;
    let missing = harness.path("missing.txt");
    let directory = harness.path("downloads");
    std::fs::create_dir_all(&directory).map_err(|error| fail("fixture", error))?;
    for destination in [&missing, &directory] {
        let (outcome, value) = envelope(
            &harness,
            &[
                "fs",
                "get",
                "gpu",
                "/srv/app/model.py",
                &destination.to_string_lossy(),
                "--json",
            ],
        )?;
        assert_eq!(outcome.status, 255, "{value}");
        want_code(&value, "TRANSFER_FAILED")?;
        assert_eq!(value["error"]["retryable"], false);
    }
    Ok(())
}

#[test]
fn path_mistakes_and_dangerous_targets_never_reach_a_transfer_tool() -> Result<(), String> {
    let harness = Harness::open("fs-refusals")?;
    harness.stub("ssh", "exit 255")?;
    harness.stub("scp", "exit 0")?;
    harness.stub("rsync", "exit 0")?;
    let file = harness.path("yes.txt");
    std::fs::write(&file, "x\n").map_err(|e| fail("fixture", e))?;
    let source = file.to_string_lossy().to_string();
    let tree = harness.path("tree");
    std::fs::create_dir_all(&tree).map_err(|e| fail("fixture", e))?;
    let tree = tree.to_string_lossy().to_string();

    for (args, code) in [
        (
            vec!["fs", "put", "gpu", &source, "'/tmp/x'", "--json"],
            "CONFIG_INVALID",
        ),
        (
            vec!["fs", "put", "gpu", "/nonexistent-rhost", "/tmp/x", "--json"],
            "CONFIG_INVALID",
        ),
        (
            vec!["fs", "put", "gpu", ".", "/tmp/x", "--json"],
            "CONFIG_INVALID",
        ),
        (
            vec!["fs", "put", "gpu", &source, "/tmp/*", "--json"],
            "SYNC_REJECTED",
        ),
        (
            vec!["fs", "sync", "gpu", &tree, "/tmp", "--delete", "--json"],
            "SYNC_REJECTED",
        ),
        (
            vec!["fs", "sync", "gpu", &tree, "~", "--delete", "--json"],
            "SYNC_REJECTED",
        ),
        (
            vec!["fs", "sync", "gpu", &tree, "/", "--delete", "--json"],
            "SYNC_REJECTED",
        ),
        (
            vec!["fs", "sync", "gpu", &tree, "/tmp/rhost-*", "--json"],
            "SYNC_REJECTED",
        ),
        (
            vec!["fs", "sync", "gpu", &tree, ".", "--json"],
            "SYNC_REJECTED",
        ),
        (
            vec!["fs", "mirror", "gpu", "/srv/app", "/", "--delete", "--json"],
            "SYNC_REJECTED",
        ),
    ] {
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(outcome.status, 255, "{args:?}");
        want_code(&value, code).map_err(|error| format!("{args:?} {error}"))?;
        assert_eq!(value["data"], serde_json::Value::Null);
    }
    harness.assert_no_remote_tool("refused file operations")?;
    Ok(())
}

#[test]
fn a_failed_transfer_keeps_the_tool_complaint_and_stays_retryable() -> Result<(), String> {
    let harness = Harness::open("fs-failed")?;
    local_host(&harness)?;
    harness.stub(
        "scp",
        "printf 'scp: /srv/app: No such file or directory\\n' >&2\nexit 1",
    )?;
    let file = harness.path("yes.txt");
    std::fs::write(&file, "x\n").map_err(|e| fail("fixture", e))?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "put",
            "gpu",
            &file.to_string_lossy(),
            "/srv/app/yes.txt",
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 255);
    want_code(&value, "TRANSFER_FAILED")?;
    assert_eq!(
        value["error"]["retryable"], true,
        "the tool refused, not the host: {value}"
    );
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("No such file or directory"),
        "the tool's own complaint is the actionable part: {value}"
    );
    Ok(())
}

#[test]
fn a_missing_local_tool_is_not_retryable() -> Result<(), String> {
    // A PATH with the stubs and nothing else: the tool this copy needs is simply
    // not on this machine, which is not the remote's fault and not retryable.
    let harness = Harness::open("fs-missing-tool")?.stubs_only_path();
    let file = harness.path("yes.txt");
    std::fs::write(&file, "x\n").map_err(|e| fail("fixture", e))?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "get",
            "gpu",
            "/tmp/rhost-missing-scp.txt",
            &file.to_string_lossy(),
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 255);
    want_code(&value, "TRANSFER_FAILED")?;
    assert_eq!(value["error"]["retryable"], false);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("not installed"),
        "{value}"
    );
    Ok(())
}

#[test]
fn a_transfer_deadline_bounds_the_local_tool() -> Result<(), String> {
    let harness = Harness::open("fs-descendant")?;
    harness.stub("ssh", "exit 255")?;
    // Process-group membership is proved with a readiness handshake in the
    // process module. This black-box case proves the public transfer deadline;
    // it must not assume the newly spawned stub was scheduled before that
    // deadline expired.
    harness.stub("scp", "sleep 5 &\nwait")?;
    let source = harness.path("input");
    std::fs::write(&source, "input").map_err(|e| fail("fixture", e))?;
    let started = std::time::Instant::now();
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "get",
            "gpu",
            "/tmp/rhost-transfer-test",
            &source.to_string_lossy(),
            "--json",
            "--timeout",
            "2s",
        ],
    )?;
    let elapsed = started.elapsed();
    assert_eq!(outcome.status, 124, "a timeout exits 124: {value}");
    want_code(&value, "REMOTE_COMMAND_TIMEOUT")?;
    assert_eq!(value["error"]["retryable"], false);
    // Well under the 5s the descendant would hold the pipes for, with room for a
    // loaded machine to start the tool.
    assert!(
        elapsed < std::time::Duration::from_millis(3500),
        "a forked descendant kept the transfer pipes open: {elapsed:?}"
    );
    Ok(())
}

#[test]
fn a_sync_reports_the_plan_the_tool_printed_and_only_deletes_when_asked() -> Result<(), String> {
    let harness = Harness::open("fs-sync")?;
    harness.stub("rsync", &rsync_stub("exit 0"))?;
    local_host(&harness)?;
    let tree = harness.path("tree");
    std::fs::create_dir_all(&tree).map_err(|e| fail("fixture", e))?;
    std::fs::write(tree.join("a.txt"), "alpha\n").map_err(|e| fail("fixture", e))?;

    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "sync",
            "gpu",
            &tree.to_string_lossy(),
            "/srv/app",
            "--dry-run",
            "--exclude",
            ".git",
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(value["data"]["backend"], "rsync");
    assert_eq!(value["data"]["dry_run"], true);
    assert_eq!(value["data"]["delete"], false);
    assert_eq!(value["data"]["files"], 2);
    assert_eq!(value["data"]["directories"], 1);
    assert_eq!(value["data"]["deletes"], 1);
    let changes = value["data"]["changes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(changes.len(), 4, "{value}");
    assert_eq!(changes[0]["action"], "create");
    assert_eq!(changes[0]["path"], "a.txt");
    assert_eq!(changes[2]["action"], "update");
    assert_eq!(changes[3]["action"], "delete");
    assert_eq!(changes[3]["itemize"], "*deleting");
    assert!(
        value["data"]["notes"][0]
            .as_str()
            .unwrap_or("")
            .contains("warning"),
        "a tool's own words are kept, not dropped: {value}"
    );
    let rsync = harness.tool_calls("rsync");
    assert_eq!(rsync.len(), 1, "one rsync invocation");
    let arguments = &rsync[0].args;
    assert!(arguments.iter().any(|a| a == "--dry-run"));
    assert!(!arguments.iter().any(|a| a == "--delete"), "{arguments:?}");
    assert_eq!(arguments[arguments.len() - 1], "gpu:/srv/app");
    assert!(
        arguments[arguments.len() - 2].ends_with("tree/"),
        "{arguments:?}"
    );
    assert!(
        arguments
            .iter()
            .any(|a| a.contains("--exclude") || a == ".git"),
        "{arguments:?}"
    );
    Ok(())
}

#[test]
fn a_destructive_sync_refuses_a_destination_that_resolves_to_the_root() -> Result<(), String> {
    let harness = Harness::open("fs-sync-resolve")?;
    harness.stub("rsync", &rsync_stub("exit 0"))?;
    local_host(&harness)?;
    let link = harness.path("root-link");
    std::os::unix::fs::symlink("/", &link).map_err(|e| fail("fixture", e))?;
    let tree = harness.path("tree");
    std::fs::create_dir_all(&tree).map_err(|e| fail("fixture", e))?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "sync",
            "gpu",
            &tree.to_string_lossy(),
            &link.to_string_lossy(),
            "--delete",
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 255, "{value}");
    want_code(&value, "SYNC_REJECTED")?;
    assert!(
        harness.tool_calls("rsync").is_empty(),
        "rsync must not run before the destination is resolved"
    );
    Ok(())
}
