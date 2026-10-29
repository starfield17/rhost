//! Structured search/read-many return revisions, without interpreting shell output.
use crate::{
    remote_fs_support::{ask, create, sha256_hex},
    support::Harness,
};
use serde_json::json;
#[test]
fn search_returns_file_hashes_and_refuses_a_changed_continuation() -> Result<(), String> {
    let h = Harness::open("native-search")?;
    let a = create(&h, "tree/a.rs", b"one\nneedle here\n")?;
    let b = create(&h, "tree/b.rs", b"needle too\n")?;
    std::os::unix::fs::symlink(&a, h.path("tree/link.rs")).map_err(|e| e.to_string())?;
    let mut q = json!({"op":"search","root":h.path("tree"),"glob":"*.rs","pattern":"needle","max_results":1,"max_matches":10,"max_bytes":4096});
    let found = ask(&h, q.clone())?;
    assert_eq!(found["items"][0]["path"], a, "{found}");
    assert_eq!(
        found["items"][0]["sha256"],
        sha256_hex(b"one\nneedle here\n")
    );
    assert_eq!(found["items"][0]["matches"][0]["line"], 2);
    assert!(found["next_cursor"].is_string(), "{found}");
    q["cursor"] = found["next_cursor"].clone();
    let next = ask(&h, q.clone())?;
    assert_eq!(next["items"][0]["path"], b, "{next}");
    assert!(next["next_cursor"].is_null());
    std::fs::write(&b, b"changed\n").map_err(|e| e.to_string())?;
    assert_eq!(ask(&h, q)?["error"], "FILE_CONFLICT");
    Ok(())
}
#[test]
fn read_many_has_an_aggregate_content_budget_and_per_file_refusals() -> Result<(), String> {
    let h = Harness::open("native-read-many")?;
    let a = create(&h, "a", b"alpha\n")?;
    let b = create(&h, "b", b"beta\n")?;
    let missing = h.path("missing");
    let read = ask(
        &h,
        json!({"op":"read_many","paths":[missing,a,b],"start":1,"lines":20,"max_bytes":6}),
    )?;
    assert_eq!(
        read["items"][0]["error"]["code"], "FILE_NOT_FOUND",
        "{read}"
    );
    assert_eq!(read["items"][1]["data"]["content"], "alpha\n");
    assert_eq!(read["unread"][0], b);
    Ok(())
}

#[test]
fn per_file_match_truncation_and_scan_incompleteness_are_distinct() -> Result<(), String> {
    let h = Harness::open("search-coverage")?;
    create(&h, "tree/a", b"needle\nneedle\n")?;
    create(&h, "tree/b", b"needle\n")?;
    let q = json!({"op":"search","root":h.path("tree"),"pattern":"needle","max_results":1,"max_matches":1,"max_bytes":1024});
    let page = ask(&h, q.clone())?;
    assert_eq!(page["items"][0]["matches_truncated"], true, "{page}");
    assert_eq!(page["incomplete"], false);
    assert!(page["next_cursor"].is_string());
    create(&h, "tree/binary", &[0xff, 0xfe])?;
    let partial = ask(&h, q)?;
    assert_eq!(partial["incomplete"], true, "{partial}");
    assert!(partial["next_cursor"].is_null());
    assert_eq!(partial["skipped"][0]["code"], "INVALID_TEXT");
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn changing_files_still_charge_the_actual_scan_read_budget() -> Result<(), String> {
    use std::{
        os::unix::process::CommandExt,
        process::{Child, Command, ExitStatus, Stdio},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant, SystemTime},
    };
    struct Writer {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Writer {
        fn finish(&mut self) -> Result<(), String> {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                thread.join().map_err(|_| "writer panicked")?;
            }
            Ok(())
        }
    }
    impl Drop for Writer {
        fn drop(&mut self) {
            let _ = self.finish();
        }
    }
    struct Group {
        child: Child,
        reaped: bool,
    }
    impl Group {
        fn terminate(&mut self) {
            // The owned leader remains unreaped, preventing numeric reuse.
            // This file-only helper never escapes its fixture process group.
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", self.child.id())])
                .status();
        }
        fn wait(&mut self) -> std::io::Result<ExitStatus> {
            let result = self.child.wait();
            self.reaped = result.is_ok();
            result
        }
    }
    impl Drop for Group {
        fn drop(&mut self) {
            if !self.reaped {
                self.terminate();
                let _ = self.wait();
            }
        }
    }
    let h = Harness::open("search-read-budget")?;
    std::fs::create_dir(h.path("tree")).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for i in 0..64 {
        let file =
            std::fs::File::create(h.path(&format!("tree/f{i:03}"))).map_err(|e| e.to_string())?;
        file.set_len(8 * 1024 * 1024).map_err(|e| e.to_string())?;
        files.push(file);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            for file in &files {
                let _ = file.set_modified(SystemTime::now());
            }
        }
    });
    let mut writer = Writer {
        stop,
        thread: Some(thread),
    };
    let payload =
        serde_json::to_vec(&json!({"op":"search","root":h.path("tree"),"pattern":"absent"}))
            .map_err(|e| e.to_string())?;
    let output_path = h.path("answer");
    let input_path = h.path("request");
    std::fs::write(&input_path, &payload).map_err(|e| e.to_string())?;
    // Linux folds a reaped child's I/O counters into its live parent's
    // signal accounting. A stopped shell retains a readable /proc/io after
    // the helper exits; the helper's own zombie /proc/io may deny access.
    let child = Command::new("/bin/sh")
            .process_group(0)
            .args(["-c", "\"$RHOST_ACCOUNT_HELPER\" --files-json < \"$RHOST_ACCOUNT_INPUT\"; status=$?; kill -STOP $$; exit \"$status\""])
            .env("RHOST_ACCOUNT_HELPER", std::env::var_os("RHOST_HELPER_BIN").ok_or("set RHOST_HELPER_BIN")?)
            .env("RHOST_ACCOUNT_INPUT", input_path)
            .env_remove("RHOST_EXPECT_BOOT_ID")
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&output_path).map_err(|e| e.to_string())?)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
    let mut group = Group {
        child,
        reaped: false,
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut answer = None;
    while Instant::now() < deadline {
        let stopped = std::fs::read_to_string(format!("/proc/{}/status", group.child.id()))
            .is_ok_and(|s| {
                s.lines().any(|line| {
                    line.strip_prefix("State:")
                        .is_some_and(|state| state.trim().starts_with('T'))
                })
            });
        if stopped
            && let Ok(bytes) = std::fs::read(&output_path)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
        {
            answer = Some(value);
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    // rchar includes all read/pread bytes, including page-cache reads and the
    // reaped helper. The shell/loader overhead remains inside the 1 MiB bound.
    let accounting = std::fs::read_to_string(format!("/proc/{}/io", group.child.id()));
    writer.finish()?;
    if answer.is_none() {
        group.terminate();
    } else {
        let continued = Command::new("kill")
            .args(["-CONT", &group.child.id().to_string()])
            .status();
        if !continued.is_ok_and(|s| s.success()) {
            group.terminate();
        }
    }
    let status = group.wait().map_err(|e| e.to_string())?;
    let answer = answer.ok_or("bounded search did not finish")?;
    assert!(status.success(), "{answer}");
    let chars = accounting
        .map_err(|e| e.to_string())?
        .lines()
        .find_map(|s| {
            s.strip_prefix("rchar: ")
                .and_then(|v| v.trim().parse::<u64>().ok())
        })
        .ok_or("read accounting missing")?;
    let scanned = answer["scanned_bytes"].as_u64().ok_or("scanned bytes")?;
    assert!(scanned <= 64 * 1024 * 1024, "{answer}");
    assert!(
        chars <= scanned + payload.len() as u64 + 1024 * 1024,
        "kernel read counter {chars} exceeds reported scan {scanned}"
    );
    assert_eq!(answer["incomplete"], true);
    Ok(())
}
