//! Full public request path with real helpers and a hermetic SSH process stand-in.
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let parent = std::fs::canonicalize("target").unwrap_or_else(|e| panic!("target: {e}"));
        let root = parent.join(format!(
            "run-cli-{}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_else(|e| panic!("time: {e}"))
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("bin")).unwrap_or_else(|e| panic!("fixture: {e}"));
        let ssh = root.join("bin/ssh");
        std::fs::write(
            &ssh,
            b"#!/bin/sh\nfor arg do last=$arg; done\nexec /bin/sh -c \"$last\"\n",
        )
        .unwrap_or_else(|e| panic!("SSH: {e}"));
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700))
            .unwrap_or_else(|e| panic!("mode: {e}"));
        let cache = root.join(format!(
            ".cache/rhost/helper/{}",
            rhost::helper::source_digest()
        ));
        let helper =
            std::env::var("RHOST_HELPER_BIN").unwrap_or_else(|_| panic!("set RHOST_HELPER_BIN"));
        let output = Command::new(helper)
            .arg("--install-cache")
            .arg(cache)
            .output()
            .unwrap_or_else(|e| panic!("install: {e}"));
        assert!(output.status.success(), "{output:?}");
        Self { root }
    }
    fn spawn(&self, request: &Value) -> std::process::Child {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rhost"))
            .args(["request", "--from", "-"])
            .env("HOME", &self.root)
            .env("RHOST_REMOTE_STATE", self.root.join("state"))
            .env("RHOST_CACHE_DIR", self.root.join("transport"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("CLI: {e}"));
        child
            .stdin
            .take()
            .unwrap_or_else(|| panic!("stdin"))
            .write_all(request.to_string().as_bytes())
            .unwrap_or_else(|e| panic!("request: {e}"));
        child
    }
    fn request(&self, request: &Value) -> Value {
        let output = self
            .spawn(request)
            .wait_with_output()
            .unwrap_or_else(|e| panic!("wait: {e}"));
        assert!(output.stderr.is_empty(), "{output:?}");
        let value: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("response: {e}: {output:?}"));
        let schema: Value = serde_json::from_str(include_str!("../schemas/result-v3.schema.json"))
            .unwrap_or_else(|e| panic!("schema: {e}"));
        let validator =
            jsonschema::validator_for(&schema).unwrap_or_else(|e| panic!("validator: {e}"));
        assert!(validator.is_valid(&value), "schema rejected {value}");
        assert_eq!(output.status.success(), value["ok"] == true);
        value
    }
}

#[test]
fn cancelling_an_observer_preserves_the_remote_run() {
    let f = Fixture::new();
    let envelope = |operation: &str, params: Value| json!({"schema_version":3,"operation":operation,"host":"gpu","params":params});
    let prepared = f.request(&envelope("helper.prepare", json!({})));
    let generation = prepared["data"]["generation"].clone();
    let expiry = prepared["data"]["remote_time_unix"]
        .as_u64()
        .unwrap_or_else(|| panic!("time"))
        + 3600;
    let key = format!("{expiry}-{}", "c".repeat(64));
    let started=f.request(&envelope("run.start",json!({"generation":generation,"key":key,"expires_unix":expiry,"cwd":f.root,
        "program":{"argv":["/bin/sh","-c","while [ ! -f release ]; do sleep 0.05; done; printf finished"]},
        "environment":{"inherit":false,"values":{}},"deadline_ms":10000})));
    assert_eq!(started["ok"], true, "{started}");
    let observer = f.spawn(&envelope(
        "run.poll",
        json!({"generation":generation,"run_id":key,"wait_ms":30000}),
    ));
    std::thread::sleep(std::time::Duration::from_millis(200));
    let signalled = Command::new("kill")
        .args(["-INT", &observer.id().to_string()])
        .status()
        .unwrap_or_else(|e| panic!("signal: {e}"));
    assert!(signalled.success());
    let stopped = observer
        .wait_with_output()
        .unwrap_or_else(|e| panic!("wait: {e}"));
    assert!(!stopped.status.success());
    let stopped: Value = serde_json::from_slice(&stopped.stdout)
        .unwrap_or_else(|e| panic!("cancel: {e}: {stopped:?}"));
    assert_eq!(stopped["error"]["code"], "REMOTE_COMMAND_CANCELLED");
    let live = f.request(&envelope(
        "run.poll",
        json!({"generation":generation,"run_id":key}),
    ));
    assert_eq!(live["data"]["run"]["status"], "running", "{live}");
    std::fs::write(f.root.join("release"), b"continue").unwrap_or_else(|e| panic!("release: {e}"));
    let mut completed = false;
    for _ in 0..20 {
        let result = f.request(&envelope(
            "run.poll",
            json!({"generation":generation,"run_id":key,"wait_ms":1000}),
        ));
        if result["data"]["run"]["status"] == "exited" {
            assert_eq!(result["data"]["run"]["completion"]["exit_code"], 0);
            assert_eq!(result["data"]["stdout"]["content"], "finished");
            completed = true;
            break;
        }
    }
    assert!(completed);
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[test]
fn public_requests_recover_completed_runs_and_refuse_a_changed_retry() {
    let f = Fixture::new();
    let envelope = |operation: &str, params: Value| json!({"schema_version":3,"operation":operation,"host":"gpu","params":params});
    let prepared = f.request(&envelope("helper.prepare", json!({})));
    assert_eq!(prepared["ok"], true, "{prepared}");
    let generation = prepared["data"]["generation"].clone();
    let expiry = prepared["data"]["remote_time_unix"]
        .as_u64()
        .unwrap_or_else(|| panic!("time"))
        + 3600;
    let key = format!("{expiry}-{}", "a".repeat(64));
    let mut params = json!({"generation":generation,"key":key,"expires_unix":expiry,"cwd":"/tmp","program":{"argv":["/bin/sh","-c","printf 'stdout'; printf 'stderr' >&2; exit 9"]},"environment":{"inherit":false,"values":{}},"wait_ms":3000});
    let completed = f.request(&envelope("run.start", params.clone()));
    assert_eq!(completed["ok"], true, "{completed}");
    assert_eq!(completed["data"]["run"]["completion"]["exit_code"], 9);
    assert_eq!(completed["data"]["stdout"]["content"], "stdout");
    assert_eq!(completed["data"]["stderr"]["content"], "stderr");
    let polled = f.request(&envelope(
        "run.poll",
        json!({"generation":generation,"run_id":key,"stdout_since":3}),
    ));
    assert_eq!(polled["ok"], true, "{polled}");
    assert_eq!(polled["data"]["stdout"]["content"], "out");
    params["wait_ms"] = json!(0);
    let retried = f.request(&envelope("run.start", params.clone()));
    assert_eq!(retried["ok"], true, "{retried}");
    params["program"] = json!({"argv":["/bin/sh","-c","exit 0"]});
    let conflict = f.request(&envelope("run.start", params));
    assert_eq!(conflict["error"]["code"], "IDEMPOTENCY_CONFLICT");
}
