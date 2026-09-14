use std::os::unix::fs::PermissionsExt;

use super::support::{Live, number, shell_quote, text, write};

#[test]
fn binary_put_get_and_literal_colon_path_round_trip() -> Result<(), String> {
    let live = Live::new("files-roundtrip")?;
    let host = live.host().to_string();
    let remote = live.remote_dir()?;
    let source = live.path("a:b.bin");
    let bytes = b"rhost\0binary\xffpayload\n";
    write(&source, bytes)?;
    let target = format!("{remote}/a:b.bin");
    let put = live.ok(&[
        "--json",
        "fs",
        "put",
        &host,
        &source.to_string_lossy(),
        &target,
        "--checksum",
    ])?;
    assert_eq!(put.value["data"]["backend"], "rsync");
    assert_eq!(put.value["data"]["checksum_verified"], true);
    let destination = live.path("download");
    std::fs::create_dir_all(&destination)
        .map_err(|error| format!("prepare download directory: {error}"))?;
    let get = live.ok(&[
        "--json",
        "fs",
        "get",
        &host,
        &target,
        &destination.to_string_lossy(),
        "--checksum",
    ])?;
    assert_eq!(get.value["data"]["checksum_verified"], true);
    let fetched = std::fs::read(destination.join("a:b.bin"))
        .map_err(|error| format!("read downloaded file: {error}"))?;
    assert_eq!(fetched, bytes);
    live.cleanup_remote(&remote)
}

#[test]
fn write_and_patch_are_cas_guarded_and_preserve_mode() -> Result<(), String> {
    let live = Live::new("files-cas")?;
    let host = live.host().to_string();
    let remote = live.remote_dir()?;
    let target = format!("{remote}/quoted ' file.txt");
    let initial = live.path("initial.txt");
    let replacement = live.path("replacement.txt");
    write(&initial, b"alpha\nbeta\n")?;
    write(&replacement, b"gamma\n")?;
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("chmod fixture: {error}"))?;
    live.ok(&[
        "--json",
        "fs",
        "write",
        &host,
        &target,
        "--from",
        &initial.to_string_lossy(),
    ])?;
    live.exec(&format!("chmod 640 -- {}", shell_quote(&target)))?;
    let read = live.ok(&["--json", "fs", "read", &host, &target])?;
    let hash = text(&read.value, "/data/sha256")?.to_string();
    live.ok(&[
        "--json",
        "fs",
        "write",
        &host,
        &target,
        "--from",
        &replacement.to_string_lossy(),
        "--if-hash",
        &hash,
    ])?;
    let mode = live.exec(&format!("stat -c %a -- {}", shell_quote(&target)))?;
    assert_eq!(
        text(&mode.value, "/data/output/stdout/content")?.trim(),
        "640"
    );
    let stale = live.error(
        "FILE_CONFLICT",
        &[
            "--json",
            "fs",
            "write",
            &host,
            &target,
            "--from",
            &initial.to_string_lossy(),
            "--if-hash",
            &hash,
        ],
    )?;
    assert_eq!(stale.output.status.code(), Some(255));
    let final_read = live.ok(&["--json", "fs", "read", &host, &target])?;
    assert_eq!(text(&final_read.value, "/data/content")?, "gamma\n");
    live.cleanup_remote(&remote)
}

#[test]
fn sync_plan_apply_and_delete_are_explicit() -> Result<(), String> {
    let live = Live::new("files-sync")?;
    let host = live.host().to_string();
    let remote = live.remote_dir()?;
    let source = live.path("tree");
    std::fs::create_dir_all(source.join("sub"))
        .map_err(|error| format!("prepare sync tree: {error}"))?;
    write(&source.join("a.txt"), b"a\n")?;
    write(&source.join("sub/b.txt"), b"b\n")?;
    live.exec(&format!(
        "printf orphan > {}/orphan.txt",
        shell_quote(&remote)
    ))?;
    let source_text = source.to_string_lossy();
    let dry = live.ok(&[
        "--json",
        "fs",
        "sync",
        &host,
        &source_text,
        &remote,
        "--dry-run",
    ])?;
    assert_eq!(dry.value["data"]["dry_run"], true);
    assert_eq!(dry.value["data"]["delete"], false);
    let applied = live.ok(&["--json", "fs", "sync", &host, &source_text, &remote])?;
    assert_eq!(applied.value["data"]["dry_run"], false);
    let orphan = live.exec(&format!("test -e {}/orphan.txt", shell_quote(&remote)))?;
    assert_eq!(number(&orphan.value, "/data/execution/exit_code")?, 0);
    live.ok(&[
        "--json",
        "fs",
        "sync",
        &host,
        &source_text,
        &remote,
        "--delete",
    ])?;
    let gone = live.exec(&format!("test ! -e {}/orphan.txt", shell_quote(&remote)))?;
    assert_eq!(number(&gone.value, "/data/execution/exit_code")?, 0);
    live.cleanup_remote(&remote)
}

#[test]
fn transfer_failures_and_resolved_root_targets_are_refused() -> Result<(), String> {
    let live = Live::new("files-failures")?;
    let host = live.host().to_string();
    let destination = live.path("missing-download");
    let failed = live.error(
        "TRANSFER_FAILED",
        &[
            "--json",
            "fs",
            "get",
            &host,
            "/tmp/rhost-file-that-does-not-exist",
            &destination.to_string_lossy(),
        ],
    )?;
    assert_eq!(failed.value["error"]["retryable"], true);

    let source = live.path("danger-tree");
    std::fs::create_dir_all(&source).map_err(|error| format!("create source: {error}"))?;
    write(&source.join("file"), b"never copied")?;
    let refused = live.error(
        "SYNC_REJECTED",
        &[
            "--json",
            "fs",
            "sync",
            &host,
            &source.to_string_lossy(),
            "/",
            "--delete",
        ],
    )?;
    assert_eq!(refused.output.status.code(), Some(255));
    Ok(())
}
