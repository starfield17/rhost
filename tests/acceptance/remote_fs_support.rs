//! Shared invocation helpers for the embedded filesystem helper's cases.
//!
//! Keeping the subprocess and filesystem plumbing here lets each case read as a
//! protocol assertion instead of a second harness.

//! Direct conformance for the embedded remote filesystem helper.
//!
//! These cases deliberately stop below the CLI: one JSON request goes to the
//! exact `PROGRAM` embedded in the binary, and one JSON answer comes back. That
//! makes filesystem edge failures diagnosable without a round trip through a
//! stubbed SSH transport. The CLI acceptance suite remains the oracle for the
//! public envelope; this oracle proves the helper's local semantics.

use crate::support::{Harness, fail};
const PROGRAM: &str = include_str!("../../src/files/backend/remote_fs.py");
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};

pub(crate) const ALPHA: &str = "YWxwaGEK";
pub(crate) const GAMMA: &str = "Z2FtbWEK";
pub(crate) const X: &str = "eAo=";

pub(crate) fn ask(harness: &Harness, request: Value) -> Result<Value, String> {
    let payload = serde_json::to_vec(&request).map_err(|error| fail("encode request", error))?;
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(PROGRAM)
        .env("HOME", harness.path("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| fail("spawn embedded helper", error))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "the helper has no stdin".to_string())?;
        stdin
            .write_all(&payload)
            .map_err(|error| fail("send request", error))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| fail("wait for helper", error))?;
    let status = output
        .status
        .code()
        .ok_or_else(|| "the helper exited by signal".to_string())?;
    if status != 0 {
        return Err(format!(
            "the helper exited {status}: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        fail(
            "helper stdout is not one JSON answer",
            format!(
                "{error}; stderr={}",
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    })
}

/// Like [`ask`], but fails instead of hanging when the helper has not answered
/// within `limit`. A regression that makes the helper block on a target (a FIFO
/// with no writer) must fail this test, never stall the whole suite.
pub(crate) fn ask_within(
    harness: &Harness,
    request: Value,
    limit: std::time::Duration,
) -> Result<Value, String> {
    let payload = serde_json::to_vec(&request).map_err(|error| fail("encode request", error))?;
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(PROGRAM)
        .env("HOME", harness.path("home"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| fail("spawn embedded helper", error))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "the helper has no stdin".to_string())?;
        stdin
            .write_all(&payload)
            .map_err(|error| fail("send request", error))?;
    }
    // The helper reads stdin to EOF before it does anything, so the write end has
    // to close here; `wait_with_output` does this for `ask`, but this helper
    // polls the child itself.
    drop(child.stdin.take());
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "the helper has no stdout".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "the helper has no stderr".to_string())?;
    let out_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let deadline = std::time::Instant::now() + limit;
    loop {
        match child
            .try_wait()
            .map_err(|error| fail("wait for helper", error))?
        {
            Some(_) => break,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err("the helper did not answer before the deadline".to_string());
            }
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    let stdout = out_reader
        .join()
        .map_err(|_| "the helper stdout reader panicked".to_string())?;
    let stderr = err_reader
        .join()
        .map_err(|_| "the helper stderr reader panicked".to_string())?;
    serde_json::from_slice(&stdout).map_err(|error| {
        fail(
            "helper stdout is not one JSON answer",
            format!("{error}; stderr={}", String::from_utf8_lossy(&stderr)),
        )
    })
}

/// Creates a FIFO, whose read end blocks until a writer appears unless the
/// opener asks for non-blocking mode.
pub(crate) fn mkfifo(harness: &Harness, relative: &str) -> Result<String, String> {
    let path = harness.path(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| fail("prepare fifo", error))?;
    }
    let status = Command::new("mkfifo")
        .arg(&path)
        .status()
        .map_err(|error| fail("run mkfifo", error))?;
    if !status.success() {
        return Err(format!("mkfifo exited {:?}", status.code()));
    }
    Ok(path.to_string_lossy().into_owned())
}

pub(crate) fn expect_error(harness: &Harness, request: Value, wanted: &str) -> Result<(), String> {
    let answer = ask(harness, request)?;
    if answer.get("error").and_then(Value::as_str) == Some(wanted) {
        return Ok(());
    }
    Err(format!("expected {wanted}, got {answer}"))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

pub(crate) fn create(harness: &Harness, relative: &str, body: &[u8]) -> Result<String, String> {
    let path = harness.path(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| fail("prepare fixture", error))?;
    }
    std::fs::write(&path, body).map_err(|error| fail("write fixture", error))?;
    Ok(path.to_string_lossy().into_owned())
}

pub(crate) fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|error| fail("read result", error))
}

pub(crate) fn mode(path: &str) -> Result<u32, String> {
    let metadata = std::fs::metadata(path).map_err(|error| fail("stat result", error))?;
    Ok(metadata.permissions().mode() & 0o777)
}

pub(crate) fn path_text(harness: &Harness, relative: &str) -> String {
    harness.path(relative).to_string_lossy().into_owned()
}

pub(crate) fn assert_no_write_temp(parent: &Path, context: &str) -> Result<(), String> {
    for entry in std::fs::read_dir(parent).map_err(|error| fail("inspect parent", error))? {
        let entry = entry.map_err(|error| fail("inspect parent entry", error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".rhost-write-") {
            return Err(format!("{context} left a helper temp: {name}"));
        }
    }
    Ok(())
}
