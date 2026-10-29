//! Preparation trust boundaries through the public JSON entry point.
use crate::support::{Harness, envelope_with_stdin, want_code};
use serde_json::{Value, json};
use std::io::Write;
use std::process::Stdio;
use std::time::{Duration, Instant};

fn local_host(h: &Harness) -> Result<(), String> {
    h.stub(
        "ssh",
        "for argument do last=$argument; done\nexec /bin/sh -c \"$last\"",
    )
}
fn request(h: &Harness, operation: &str) -> Result<Value, String> {
    let input = json!({"schema_version":3,"operation":operation,"host":"gpu","params":{}});
    let (_, value) =
        envelope_with_stdin(h, &["request", "--from", "-"], input.to_string().as_bytes())?;
    Ok(value)
}

#[test]
fn bootstrap_preserves_known_ssh_failure_codes() -> Result<(), String> {
    for (diagnostic, code, retryable) in [
        ("Permission denied (publickey).", "SSH_AUTH_FAILED", false),
        ("Host key verification failed.", "HOST_KEY_FAILED", false),
        (
            "ssh: Could not resolve hostname example-host",
            "HOST_UNKNOWN",
            false,
        ),
        (
            "ssh: connect to host example-host port 22: Connection refused",
            "SSH_UNREACHABLE",
            true,
        ),
        ("opaque disconnect", "REMOTE_EXECUTION_UNKNOWN", false),
    ] {
        let h = Harness::open(code)?;
        h.stub(
            "ssh",
            &format!("printf '%s\\n' '{diagnostic}' >&2\nexit 255"),
        )?;
        let value = request(&h, "metadata")?;
        want_code(&value, code)?;
        assert_eq!(value["error"]["retryable"], retryable, "{value}");
        assert!(!h.path("home/.cache/rhost").exists());
    }
    Ok(())
}

#[test]
fn missing_target_probe_tool_is_a_dependency_refusal() -> Result<(), String> {
    let h = Harness::open("missing-uname")?;
    local_host(&h)?;
    h.stub("uname", "exit 127")?;
    want_code(&request(&h, "metadata")?, "REMOTE_DEPENDENCY_MISSING")?;
    assert!(!h.path("home/.cache/rhost").exists());
    Ok(())
}

#[test]
fn nonregular_configured_build_inputs_are_refused_without_blocking() -> Result<(), String> {
    let h = Harness::open("compiler-fifo")?;
    local_host(&h)?;
    h.stub(
        "uname",
        "case $1 in -s) echo Linux;; -m) echo x86_64;; esac",
    )?;
    let fifo = crate::remote_fs_support::mkfifo(&h, "compiler")?;
    h.write("builtins", "library", None)?;
    h.write("linker", "linker", None)?;
    std::fs::create_dir(h.path("sysroot")).map_err(|e| e.to_string())?;
    let mut command = h.command(&["request", "--from", "-"])?;
    command
        .env_remove("RHOST_HELPER_BIN")
        .env("RHOST_HELPER_CC", fifo)
        .env("RHOST_HELPER_SYSROOT", h.path("sysroot"))
        .env("RHOST_HELPER_BUILTINS", h.path("builtins"))
        .env("RHOST_HELPER_LD", h.path("linker"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("stdin")?
        .write_all(br#"{"schema_version":3,"operation":"helper.prepare","host":"gpu","params":{}}"#)
        .map_err(|e| e.to_string())?;
    let limit = Instant::now() + Duration::from_secs(3);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err("configured FIFO input blocked preparation".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    want_code(&value, "CONFIG_INVALID")?;
    assert!(!h.path("home/.cache/rhost").exists());
    Ok(())
}

#[test]
fn configured_linker_keeps_its_driver_name() -> Result<(), String> {
    let h = Harness::open("linker-driver")?;
    local_host(&h)?;
    h.stub(
        "uname",
        "case $1 in -s) echo Linux;; -m) echo x86_64;; esac",
    )?;
    h.write(
        "compiler",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > {}\nexit 8\n",
            crate::support::shell_quote(&h.path("arguments").to_string_lossy())
        ),
        Some(0o700),
    )?;
    h.write("builtins", "library", None)?;
    h.write("lld", "linker", None)?;
    std::os::unix::fs::symlink(h.path("lld"), h.path("ld.lld")).map_err(|e| e.to_string())?;
    std::fs::create_dir(h.path("sysroot")).map_err(|e| e.to_string())?;
    let mut command = h.command(&["request", "--from", "-"])?;
    command
        .env_remove("RHOST_HELPER_BIN")
        .env("RHOST_HELPER_CC", h.path("compiler"))
        .env("RHOST_HELPER_SYSROOT", h.path("sysroot"))
        .env("RHOST_HELPER_BUILTINS", h.path("builtins"))
        .env("RHOST_HELPER_LD", h.path("ld.lld"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("stdin")?
        .write_all(br#"{"schema_version":3,"operation":"helper.prepare","host":"gpu","params":{}}"#)
        .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    want_code(&value, "HELPER_PREPARATION_FAILED")?;
    let argv = std::fs::read_to_string(h.path("arguments")).map_err(|e| e.to_string())?;
    assert!(
        argv.lines()
            .any(|s| s == format!("--ld-path={}", h.path("ld.lld").display())),
        "{argv}"
    );
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn same_source_cached_image_must_match_the_supplied_bytes() -> Result<(), String> {
    use std::fs::OpenOptions;
    let h = Harness::open("cache-image")?;
    let cache = h.path("helper-cache");
    let state = h.path("remote-state");
    let h = h
        .env("RHOST_HELPER_CACHE_DIR", &cache.to_string_lossy())
        .env("RHOST_REMOTE_STATE", &state.to_string_lossy());
    local_host(&h)?;
    let prepared = request(&h, "helper.prepare")?;
    assert_eq!(prepared["ok"], true, "{prepared}");
    let helper = &prepared["data"]["helper"];
    let source = helper["source_digest"].as_str().ok_or("source")?;
    let hash = helper["binary_sha256"].as_str().ok_or("image")?;
    let executable = cache.join(format!(
        "p1/{source}/linux-{}/supplied-{hash}/rhost-helper",
        std::env::consts::ARCH
    ));
    OpenOptions::new()
        .append(true)
        .open(&executable)
        .map_err(|e| e.to_string())?
        .write_all(b"different image, identical source identifier")
        .map_err(|e| e.to_string())?;
    want_code(&request(&h, "metadata")?, "HELPER_INCOMPATIBLE")?;
    let repaired = request(&h, "helper.prepare")?;
    assert_eq!(repaired["ok"], true, "{repaired}");
    assert_eq!(repaired["data"]["helper"]["binary_sha256"], hash);
    Ok(())
}
