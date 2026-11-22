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
    let input = json!({"schema_version":5,"operation":operation,"host":"gpu","params":{}});
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
        .write_all(br#"{"schema_version":5,"operation":"helper.prepare","host":"gpu","params":{}}"#)
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
        .write_all(br#"{"schema_version":5,"operation":"helper.prepare","host":"gpu","params":{}}"#)
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
fn a_warm_native_file_operation_uses_one_ssh_invocation() -> Result<(), String> {
    let h = Harness::open("warm-helper")?;
    local_host(&h)?;
    let state = h.path("remote-state").to_string_lossy().into_owned();
    let h = h.env("RHOST_REMOTE_STATE", &state);
    assert_eq!(request(&h, "helper.prepare")?["ok"], true);
    h.write("content", "observed\n", None)?;
    let before = h.calls().iter().filter(|c| c.program == "ssh").count();
    let payload = json!({"schema_version":5,"operation":"fs.read","host":"gpu","params":{"path":h.path("content")}});
    let (_, result) = envelope_with_stdin(
        &h,
        &["request", "--from", "-"],
        payload.to_string().as_bytes(),
    )?;
    assert_eq!(result["data"]["content"], "observed\n", "{result}");
    let after = h.calls().iter().filter(|c| c.program == "ssh").count();
    assert_eq!(after - before, 1, "{}", h.calls_text());
    Ok(())
}

#[test]
fn an_unrecognized_cached_executable_does_not_trigger_automatic_resubmission() -> Result<(), String>
{
    let h = Harness::open("unrecognized-helper")?;
    local_host(&h)?;
    let cache = h.path("helper-cache");
    let state = h.path("remote-state");
    let h = h
        .env("RHOST_HELPER_CACHE_DIR", &cache.to_string_lossy())
        .env("RHOST_REMOTE_STATE", &state.to_string_lossy());
    let prepared = request(&h, "helper.prepare")?;
    assert_eq!(prepared["ok"], true, "{prepared}");
    let helper = &prepared["data"]["helper"];
    let image = cache.join(format!(
        "p1/{}/{}-{}/supplied-{}/rhost-helper",
        helper["source_digest"].as_str().ok_or("source")?,
        helper["os"].as_str().ok_or("os")?,
        helper["arch"].as_str().ok_or("arch")?,
        helper["binary_sha256"].as_str().ok_or("sha")?
    ));
    std::fs::remove_file(&image).map_err(|e| e.to_string())?;
    h.write("replacement", "#!/bin/sh\nexit 0\n", Some(0o700))?;
    std::fs::rename(h.path("replacement"), &image).map_err(|e| e.to_string())?;
    let before = h.calls().iter().filter(|c| c.program == "ssh").count();
    let target = h.path("must-stay-absent");
    let payload = json!({"schema_version":5,"operation":"fs.write","host":"gpu","params":{"path":target,"content":"effect"}});
    let (_, result) = envelope_with_stdin(
        &h,
        &["request", "--from", "-"],
        payload.to_string().as_bytes(),
    )?;
    assert_eq!(result["ok"], false, "{result}");
    assert!(!target.exists());
    assert_eq!(
        h.calls().iter().filter(|c| c.program == "ssh").count() - before,
        1
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

#[test]
#[cfg(target_os = "linux")]
fn release_helper_checks_the_sidecar_and_reuses_only_matching_cached_bytes() -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let h = Harness::open("release-helper")?;
    local_host(&h)?;
    let cache = h.path("helper-cache");
    let state = h.path("remote-state");
    let helper = std::env::var("RHOST_HELPER_BIN").map_err(|error| error.to_string())?;
    let version = env!("CARGO_PKG_VERSION");
    let target = format!("linux-{}", std::env::consts::ARCH);
    let asset = format!("rhost-helper_{version}_{target}");
    let url = format!("https://github.com/starfield17/rhost/releases/download/v{version}/{asset}");
    h.stub(
        "curl",
        &format!(
            "for argument do last=$argument; done\ncase \"$last\" in\n{url}.sha256) exec cat sidecar;;\n{url}) exec cat {};;\n*) exit 2;;\nesac",
            crate::support::shell_quote(&helper)
        ),
    )?;
    let h = h
        .env("RHOST_NO_SOURCE_BUILD", "1")
        .env("RHOST_HELPER_CACHE_DIR", &cache.to_string_lossy())
        .env("RHOST_REMOTE_STATE", &state.to_string_lossy());
    let invoke = |subcommand: &str, offline: bool| -> Result<Value, String> {
        let mut child = h.command(&["helper", subcommand, "gpu", "--json"])?;
        child
            .env_remove("RHOST_HELPER_BIN")
            .env_remove("RHOST_HELPER_CC");
        if offline {
            child.env("RHOST_NO_DOWNLOAD", "1");
        } else {
            child.env_remove("RHOST_NO_DOWNLOAD");
        }
        let output = child.output().map_err(|error| error.to_string())?;
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())
    };
    let calls = || {
        h.calls()
            .into_iter()
            .filter(|call| call.program == "curl")
            .collect::<Vec<_>>()
    };
    want_code(&invoke("prepare", true)?, "HELPER_PREPARATION_FAILED")?;
    assert!(calls().is_empty());
    h.write("sidecar", &format!("{}  {asset}\n", "0".repeat(64)), None)?;
    want_code(&invoke("prepare", false)?, "HELPER_INCOMPATIBLE")?;
    assert!(!cache.exists());
    let downloaded = calls();
    assert_eq!(downloaded.len(), 2);
    assert_eq!(downloaded[0].args.last(), Some(&format!("{url}.sha256")));
    assert_eq!(downloaded[1].args.last(), Some(&url));
    for call in downloaded {
        for flag in ["--proto", "--proto-redir"] {
            assert!(call.args.windows(2).any(|args| args == [flag, "=https"]));
        }
    }

    let bytes = std::fs::read(helper).map_err(|error| error.to_string())?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    h.write("sidecar", &format!("{digest}  {asset}\n"), None)?;
    let prepared = invoke("prepare", false)?;
    assert_eq!(prepared["ok"], true, "{prepared}");
    let identity = &prepared["data"]["helper"];
    assert_eq!(identity["binary_sha256"], digest);
    let source = identity["source_digest"].as_str().ok_or("source digest")?;
    let path = cache.join(format!(
        "p1/{source}/{target}/release-v{version}-{digest}/rhost-helper"
    ));
    assert_eq!(calls().len(), 4);
    let cached = invoke("status", true)?;
    assert_eq!(
        cached["data"]["helper"]["binary_sha256"], digest,
        "{cached}"
    );
    assert_eq!(calls().len(), 4);
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut image| image.write_all(b"mismatched cached image"))
        .map_err(|error| error.to_string())?;
    want_code(&invoke("status", true)?, "HELPER_INCOMPATIBLE")?;
    assert_eq!(calls().len(), 4);
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn offline_pack_prepares_and_corrupt_pack_cannot_fall_back_to_cached_or_supplied_bytes()
-> Result<(), String> {
    let h = Harness::open("offline-pack")?;
    local_host(&h)?;
    h.stub("cc", "exit 99")?;
    h.stub("curl", "exit 99")?;
    let abi = format!("linux-{}", std::env::consts::ARCH);
    let source = rhost::helper::source_digest();
    let helper = std::env::var_os("RHOST_HELPER_BIN").ok_or("set RHOST_HELPER_BIN")?;
    let bytes = std::fs::read(helper).map_err(|e| e.to_string())?;
    let sha = crate::remote_fs_support::sha256_hex(&bytes);
    let directory = h.path(&format!("pack/{abi}/{source}/{sha}"));
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let image = directory.join("rhost-helper");
    std::fs::write(&image, bytes).map_err(|e| e.to_string())?;
    std::fs::write(
        directory.join("rhost-helper.sha256"),
        format!("{sha}  rhost-helper\n"),
    )
    .map_err(|e| e.to_string())?;
    h.write(
        "pack/manifest.json",
        &json!({"helpers":[{"target":abi,"source_digest":source,"binary_sha256":sha}]}).to_string(),
        None,
    )?;
    let pack = h.path("pack");
    let state = h.path("remote-state");
    let h = h
        .env("RHOST_HELPER_BIN", "missing-supplied-image")
        .env("RHOST_HELPER_PACK_DIR", &pack.to_string_lossy())
        .env("RHOST_REMOTE_STATE", &state.to_string_lossy())
        .env("RHOST_NO_DOWNLOAD", "1")
        .env("RHOST_NO_SOURCE_BUILD", "1");
    let prepared = request(&h, "helper.prepare")?;
    assert_eq!(
        prepared["data"]["helper"]["binary_sha256"], sha,
        "{prepared}"
    );
    assert_eq!(
        request(&h, "metadata")?["data"]["helper"]["binary_sha256"],
        sha
    );
    std::fs::write(image, b"corrupt").map_err(|e| e.to_string())?;
    want_code(&request(&h, "helper.prepare")?, "HELPER_INCOMPATIBLE")?;
    assert!(h.tool_calls("cc").is_empty());
    assert!(h.tool_calls("curl").is_empty());
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn fixed_helper_verifies_abi_and_pin_without_writing_a_helper_cache() -> Result<(), String> {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let h = Harness::open("fixed-helper")?;
    local_host(&h)?;
    h.stub("uname", "case $1 in -s) echo Linux;; *) exit 99;; esac")?;
    let abi = format!("linux-{}", std::env::consts::ARCH);
    let helper = std::env::var_os("RHOST_HELPER_BIN").ok_or("set RHOST_HELPER_BIN")?;
    let bytes = std::fs::read(helper).map_err(|e| e.to_string())?;
    let sha = crate::remote_fs_support::sha256_hex(&bytes);
    let image = h.path("home/fixed helper");
    std::fs::write(&image, bytes).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o555))
        .map_err(|e| e.to_string())?;
    let state = h.path("writable-state");
    let h = h
        .env("RHOST_REMOTE_HELPER_PATH", "~/fixed helper")
        .env("RHOST_HELPER_SHA256", &sha)
        .env("RHOST_TARGET_ABI", &abi)
        .env("RHOST_REMOTE_STATE", &state.to_string_lossy())
        .env("RHOST_HELPER_CACHE_DIR", "invalid-cache-root")
        .env("RHOST_NO_UPLOAD", "1");
    let prepared = request(&h, "helper.prepare")?;
    assert_eq!(
        prepared["data"]["helper"]["binary_sha256"], sha,
        "{prepared}"
    );
    assert!(state.is_dir());
    let content = h.path("content");
    h.write("content", "fixed helper read\n", None)?;
    let payload =
        json!({"schema_version":5,"operation":"fs.read","host":"gpu","params":{"path":content}});
    let (_, read) = envelope_with_stdin(
        &h,
        &["request", "--from", "-"],
        payload.to_string().as_bytes(),
    )?;
    assert_eq!(read["data"]["content"], "fixed helper read\n", "{read}");
    let link = h.path("home/helper-link");
    symlink(&image, &link).map_err(|e| e.to_string())?;
    let h = h.env("RHOST_REMOTE_HELPER_PATH", "~/helper-link");
    want_code(&request(&h, "helper.prepare")?, "HELPER_INCOMPATIBLE")?;
    let h = h.env("RHOST_REMOTE_HELPER_PATH", "~/fixed helper");
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o775))
        .map_err(|e| e.to_string())?;
    want_code(&request(&h, "helper.prepare")?, "HELPER_INCOMPATIBLE")?;
    std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o555))
        .map_err(|e| e.to_string())?;
    let h = h.env("RHOST_TARGET_ABI", "linux-armv7-eabihf");
    want_code(&request(&h, "helper.prepare")?, "HELPER_INCOMPATIBLE")?;
    h.write(
        "home/wrong-image",
        "#!/bin/sh\ntouch executed\n",
        Some(0o755),
    )?;
    let h = h
        .env("RHOST_REMOTE_HELPER_PATH", "~/wrong-image")
        .env("RHOST_TARGET_ABI", &abi);
    want_code(&request(&h, "helper.prepare")?, "HELPER_INCOMPATIBLE")?;
    assert!(!h.path("executed").exists());
    assert!(!h.path("home/.cache/rhost").exists());
    let uname = h.tool_calls("uname");
    assert!(!uname.is_empty());
    assert!(uname.iter().all(|c| c.args == ["-s"]));
    Ok(())
}
