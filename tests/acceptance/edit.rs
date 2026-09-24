//! The remote editing surface: bounded reads, hash-guarded writes and patches.
//!
//! The helper runs on the "remote" side — here, the local host stub — and its
//! answers are the CLI's data. A host without python3 gets the dependency answer
//! instead, which is why `python3_available` exists.

use crate::hosts::{local_host, scp_stub};
use crate::support::{Harness, envelope, fail, python3_available, want_code};
use std::os::unix::fs::symlink;

#[test]
fn an_unusable_remote_python_is_a_dependency_failure_before_any_edit() -> Result<(), String> {
    let harness = Harness::open("fs-python-unusable")?;
    local_host(&harness)?;
    // Keep this SSH stand-in below the login shell. The bash stand-in restores
    // the fixture PATH after that shell's startup files have run.
    harness.stub(
        "ssh",
        "for argument in \"$@\"; do remote=\"$argument\"; done\nexec /bin/bash -c \"$remote\"",
    )?;
    harness.stub(
        "bash",
        &format!(
            "if [ \"$1\" = -lc ]; then shift; exec /bin/bash -lc \"export PATH={}:{}; $1\"; fi\nexec /bin/bash \"$@\"",
            harness.path("stubs").display(),
            crate::support::SYSTEM_PATH,
        ),
    )?;
    harness.stub("python3", "exit 42")?;
    let source = harness.path("source.txt");
    let target = harness.path("remote/new.txt");
    std::fs::write(&source, "contents\n").map_err(|error| fail("source", error))?;
    std::fs::create_dir_all(harness.path("remote")).map_err(|error| fail("parent", error))?;

    let (_, doctor) = envelope(&harness, &["doctor", "gpu", "--json"])?;
    assert_eq!(
        doctor["data"]["capabilities"]["python3"],
        true,
        "{doctor}; calls: {}",
        harness.calls_text()
    );

    let (outcome, answer) = envelope(
        &harness,
        &[
            "fs",
            "write",
            "gpu",
            &target.to_string_lossy(),
            "--from",
            &source.to_string_lossy(),
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 255);
    want_code(&answer, "REMOTE_DEPENDENCY_MISSING")?;
    assert!(
        !target.exists(),
        "the failing interpreter must not edit a file"
    );
    Ok(())
}

#[test]
fn reading_and_writing_a_remote_file_needs_the_hash_it_was_read_with() -> Result<(), String> {
    let harness = Harness::open("fs-edit")?;
    local_host(&harness)?;
    let target = harness.path("remote/text.txt");
    std::fs::create_dir_all(harness.path("remote")).map_err(|e| fail("fixture", e))?;
    std::fs::write(&target, "alpha\nbeta\n").map_err(|e| fail("fixture", e))?;
    let source = harness.path("local.txt");
    std::fs::write(&source, "gamma\n").map_err(|e| fail("fixture", e))?;
    let target = target.to_string_lossy().to_string();
    let source = source.to_string_lossy().to_string();

    if !python3_available() {
        let (outcome, value) = envelope(&harness, &["fs", "read", "gpu", &target, "--json"])?;
        assert_eq!(outcome.status, 255);
        want_code(&value, "REMOTE_DEPENDENCY_MISSING")?;
        return Ok(());
    }

    let (outcome, value) = envelope(&harness, &["fs", "read", "gpu", &target, "--json"])?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(value["data"]["content"], "alpha\nbeta\n");
    assert_eq!(value["data"]["total_lines"], 2);
    assert_eq!(value["data"]["truncated"], false);
    let hash = value["data"]["sha256"].as_str().unwrap_or("").to_string();
    assert_eq!(hash.len(), 64, "{value}");

    // Creating a file needs no precondition.
    let created = harness.path("remote/new.txt");
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "write",
            "gpu",
            &created.to_string_lossy(),
            "--from",
            &source,
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(value["data"]["bytes"], 6);
    assert_eq!(
        std::fs::read_to_string(&created).map_err(|e| fail("read back", e))?,
        "gamma\n"
    );

    // Replacing one requires the hash from the read.
    let (outcome, value) = envelope(
        &harness,
        &["fs", "write", "gpu", &target, "--from", &source, "--json"],
    )?;
    assert_eq!(outcome.status, 255, "{value}");
    want_code(&value, "HASH_REQUIRED")?;

    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "write",
            "gpu",
            &target,
            "--from",
            &source,
            "--if-hash",
            &hash,
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(
        std::fs::read_to_string(&target).map_err(|e| fail("read back", e))?,
        "gamma\n"
    );

    // The same hash cannot be replayed: the file is no longer the one it named.
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "write",
            "gpu",
            &target,
            "--from",
            &source,
            "--if-hash",
            &hash,
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 255, "{value}");
    want_code(&value, "FILE_CONFLICT")?;
    Ok(())
}

#[test]
fn a_patch_is_applied_once_and_refused_when_its_hash_no_longer_describes_the_file()
-> Result<(), String> {
    let harness = Harness::open("fs-patch")?;
    local_host(&harness)?;
    if !python3_available() {
        return Ok(());
    }
    let target = harness.path("remote/patch.txt");
    std::fs::create_dir_all(harness.path("remote")).map_err(|e| fail("fixture", e))?;
    std::fs::write(&target, "alpha\nbeta\n").map_err(|e| fail("fixture", e))?;
    let target = target.to_string_lossy().to_string();
    let (_, read) = envelope(&harness, &["fs", "read", "gpu", &target, "--json"])?;
    let hash = read["data"]["sha256"].as_str().unwrap_or("").to_string();

    let document = harness.path("patch.json");
    std::fs::write(
        &document,
        format!(
            "{{\"sha256\":\"{hash}\",\"edits\":[{{\"start\":2,\"end\":2,\"text\":\"gamma\\n\"}}]}}"
        ),
    )
    .map_err(|e| fail("fixture", e))?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "patch",
            "gpu",
            &target,
            "--patch",
            &document.to_string_lossy(),
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(
        std::fs::read_to_string(&target).map_err(|e| fail("read back", e))?,
        "alpha\ngamma\n"
    );

    // Applying it again must be refused, not applied twice.
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "patch",
            "gpu",
            &target,
            "--patch",
            &document.to_string_lossy(),
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 255);
    want_code(&value, "FILE_CONFLICT")?;

    // A patch document without a hash is a local configuration answer.
    let empty = harness.path("empty.json");
    std::fs::write(
        &empty,
        "{\"edits\":[{\"start\":1,\"end\":1,\"text\":\"x\"}]}",
    )
    .map_err(|e| fail("fixture", e))?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "patch",
            "gpu",
            &target,
            "--patch",
            &empty.to_string_lossy(),
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 255);
    want_code(&value, "CONFIG_INVALID")?;
    Ok(())
}

#[test]
fn a_batch_reports_every_entry_and_fails_the_process_only_for_failed_entries() -> Result<(), String>
{
    let harness = Harness::open("fs-batch")?;
    local_host(&harness)?;
    harness.stub(
        "scp",
        &scp_stub(
            "case \"$src\" in *missing*)\n  printf 'scp: /srv/missing.txt: No such file or directory\\n' >&2\n  exit 1\n  ;;\nesac\nexit 0",
        ),
    )?;
    let good = harness.path("good.txt");
    std::fs::write(&good, "good\n").map_err(|e| fail("fixture", e))?;
    let download = harness.path("download.txt");
    let manifest = harness.path("manifest.json");
    std::fs::write(
        &manifest,
        format!(
            "[{{\"op\":\"put\",\"source\":\"{}\",\"destination\":\"/srv/good.txt\"}},{{\"op\":\"get\",\"source\":\"/srv/missing.txt\",\"destination\":\"{}\"}}]",
            good.to_string_lossy(),
            download.to_string_lossy()
        ),
    )
    .map_err(|e| fail("fixture", e))?;

    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "batch",
            "gpu",
            "--manifest",
            &manifest.to_string_lossy(),
            "--json",
        ],
    )?;
    assert_eq!(
        outcome.status, 255,
        "a batch with a failed entry is not a clean run: {value}"
    );
    assert_eq!(
        value["ok"], true,
        "the batch itself ran to the end: {value}"
    );
    assert_eq!(value["data"]["succeeded"], 1);
    assert_eq!(value["data"]["failed"], 1);
    let items = value["data"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["ok"], true);
    assert_eq!(items[0]["op"], "put");
    assert_eq!(items[0]["data"]["backend"], "scp");
    assert_eq!(items[1]["ok"], false);
    assert_eq!(items[1]["op"], "get");
    assert!(items[1]["error"]["code"].is_string(), "{value}");
    assert!(
        items[1].get("data").is_none(),
        "a failed entry carries no data: {value}"
    );
    Ok(())
}

#[test]
fn helper_backed_edits_validate_locally_before_any_remote_call() -> Result<(), String> {
    let harness = Harness::open("fs-edit-validation")?;
    harness.stub("ssh", "exit 99")?;
    let patch = harness.path("patch.json");
    std::fs::write(&patch, "{\"edits\":[]}").map_err(|e| fail("fixture", e))?;
    for (args, code) in [
        (
            vec!["fs", "read", "gpu", "/tmp/x", "--max-bytes", "0", "--json"],
            "USAGE_ERROR",
        ),
        (
            vec!["fs", "read", "gpu", "/tmp/x", "--lines", "0", "--json"],
            "USAGE_ERROR",
        ),
        (
            vec![
                "fs",
                "read",
                "gpu",
                "/tmp/x",
                "--max-bytes",
                "99999999",
                "--json",
            ],
            "USAGE_ERROR",
        ),
        (
            vec!["fs", "read", "gpu", "'/tmp/x'", "--json"],
            "CONFIG_INVALID",
        ),
        (
            vec!["fs", "write", "gpu", "/tmp/x", "--mode", "abc", "--json"],
            "CONFIG_INVALID",
        ),
        (
            vec![
                "fs",
                "patch",
                "gpu",
                "/tmp/x",
                "--patch",
                &patch.to_string_lossy(),
                "--json",
            ],
            "CONFIG_INVALID",
        ),
    ] {
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(outcome.status, 255, "{args:?}");
        want_code(&value, code).map_err(|error| format!("{args:?} {error}"))?;
    }
    harness.assert_no_remote_tool("locally refused edits")?;
    Ok(())
}

#[test]
fn a_write_body_is_read_locally_and_bounded() -> Result<(), String> {
    let harness = Harness::open("fs-write-body")?;
    harness.stub("ssh", "exit 99")?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "fs",
            "write",
            "gpu",
            "/tmp/x",
            "--from",
            "/nonexistent-rhost",
            "--json",
        ],
    )?;
    assert_eq!(outcome.status, 255);
    want_code(&value, "CONFIG_INVALID")?;
    harness.assert_no_remote_tool("a missing write body")
}

#[test]
fn the_editing_surface_refuses_symlinked_targets_without_reading_or_writing_through()
-> Result<(), String> {
    let harness = Harness::open("fs-edit-symlink")?;
    local_host(&harness)?;
    if !python3_available() {
        return Ok(());
    }
    let root = harness.path("symlink");
    std::fs::create_dir_all(&root).map_err(|error| fail("prepare symlink fixture", error))?;
    let real = root.join("real.txt");
    let replacement = root.join("replacement.txt");
    let document = root.join("link-patch.json");
    std::fs::write(&real, "alpha\n").map_err(|error| fail("write real", error))?;
    std::fs::write(&replacement, "gamma\n").map_err(|error| fail("write body", error))?;
    let link = root.join("link.txt");
    symlink(&real, &link).map_err(|error| fail("create link", error))?;
    let link = link.to_string_lossy().into_owned();
    let replacement = replacement.to_string_lossy().into_owned();
    let document_text = document.to_string_lossy().into_owned();
    let (_, value) = envelope(
        &harness,
        &["fs", "read", "gpu", &real.to_string_lossy(), "--json"],
    )?;
    let hash = value["data"]["sha256"]
        .as_str()
        .ok_or_else(|| "read returned no hash".to_string())?
        .to_string();
    std::fs::write(
        &document,
        format!(
            "{{\"sha256\":\"{hash}\",\"edits\":[{{\"start\":1,\"end\":1,\"text\":\"nope\\n\"}}]}}"
        ),
    )
    .map_err(|error| fail("write patch", error))?;

    for args in [
        vec!["fs", "read", "gpu", link.as_str(), "--json"],
        vec![
            "fs",
            "write",
            "gpu",
            link.as_str(),
            "--from",
            replacement.as_str(),
            "--json",
        ],
        vec![
            "fs",
            "patch",
            "gpu",
            link.as_str(),
            "--patch",
            document_text.as_str(),
            "--json",
        ],
    ] {
        let arguments = args.to_vec();
        let (outcome, value) = envelope(&harness, &arguments)?;
        assert_eq!(outcome.status, 255, "{arguments:?}: {value}");
        want_code(&value, "INVALID_TARGET")?;
    }
    assert_eq!(
        std::fs::read_to_string(&real).map_err(|error| fail("read back", error))?,
        "alpha\n"
    );
    for entry in std::fs::read_dir(&root).map_err(|error| fail("inspect link root", error))? {
        let name = entry
            .map_err(|error| fail("inspect link root entry", error))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        assert!(
            !name.starts_with(".rhost-write-"),
            "a refused symlink edit left a helper temp: {name}"
        );
    }
    Ok(())
}
