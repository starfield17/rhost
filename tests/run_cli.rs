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

fn message(operation: &str, params: Value) -> Value {
    json!({"schema_version":3,"operation":operation,"host":"gpu","params":params})
}
fn namespace(f: &Fixture) -> (Value, u64) {
    let v = f.request(&message("helper.prepare", json!({})));
    assert_eq!(v["ok"], true, "{v}");
    (
        v["data"]["generation"].clone(),
        v["data"]["remote_time_unix"]
            .as_u64()
            .unwrap_or_else(|| panic!("time"))
            + 3600,
    )
}
fn launch(f: &Fixture, g: &Value, expiry: u64, script: &str) -> Value {
    json!({"generation":g,"key":format!("{expiry}-{}","e".repeat(64)),"expires_unix":expiry,"cwd":f.root,
        "program":{"argv":["/bin/sh","-c",script]},"environment":{"inherit":false,"values":{}},"wait_ms":1000,"deadline_ms":15000})
}
fn finished(f: &Fixture, g: &Value, key: &Value) -> Value {
    for _ in 0..100 {
        let result = f.request(&message(
            "run.poll",
            json!({"generation":g,"run_id":key,"wait_ms":1000}),
        ));
        assert_eq!(result["ok"], true, "{result}");
        if result["data"]["run"]["status"] == "exited" && result["data"]["stdout"]["open"] == false
        {
            return result["data"].clone();
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("completion and closed streams were not observed");
}

#[test]
fn output_quota_preserves_source_counts_and_explicit_gaps() {
    let f = Fixture::new();
    let (g, expiry) = namespace(&f);
    let params = launch(
        &f,
        &g,
        expiry,
        "dd if=/dev/zero bs=1048576 count=65 2>/dev/null",
    );
    let key = params["key"].clone();
    assert_eq!(f.request(&message("run.start", params))["ok"], true);
    let done = finished(&f, &g, &key);
    assert_eq!(done["run"]["completion"]["exit_code"], 0);
    assert_eq!(done["stdout"]["observed_bytes"], 65 * 1048576);
    assert_eq!(done["stdout"]["next"], 4096);
    assert_eq!(done["stdout"]["incomplete"], true);
    let log = f
        .root
        .join("state/runs")
        .join(key.as_str().unwrap_or_else(|| panic!("key")))
        .join("stdout");
    assert_eq!(
        std::fs::metadata(log)
            .unwrap_or_else(|e| panic!("log: {e}"))
            .len(),
        64 * 1048576
    );
}

#[test]
fn run_log_write_failures_keep_observed_counts_and_gaps() {
    let f = Fixture::new();
    let (g, expiry) = namespace(&f);
    // Existing cached bytes are prepared before imposing a per-file write
    // limit. Ignored SIGXFSZ makes write return EFBIG, the monitor's normal
    // storage-error path, while small completion records can still persist.
    std::fs::write(f.root.join("bin/ssh"), b"#!/bin/sh\ntrap '' XFSZ\nulimit -f 64\nfor arg do last=$arg; done\nexec /bin/sh -c \"$last\"\n").unwrap_or_else(|e| panic!("SSH: {e}"));
    let params = launch(
        &f,
        &g,
        expiry,
        "dd if=/dev/zero bs=1048576 count=1 2>/dev/null",
    );
    let key = params["key"].clone();
    assert_eq!(f.request(&message("run.start", params))["ok"], true);
    let done = finished(&f, &g, &key);
    assert_eq!(done["run"]["completion"]["exit_code"], 0);
    assert_eq!(done["stdout"]["observed_bytes"], 1048576);
    assert_eq!(done["stdout"]["incomplete"], true);
    let log = f
        .root
        .join("state/runs")
        .join(key.as_str().unwrap_or_else(|| panic!("key")))
        .join("stdout");
    let kept = std::fs::metadata(log)
        .unwrap_or_else(|e| panic!("log: {e}"))
        .len();
    assert!(
        kept > 0 && kept <= 65536,
        "retained {kept} after write failure"
    );
}

#[test]
fn expiry_survives_pruning_and_namespace_guards_refuse_replacement_state() {
    let f = Fixture::new();
    let (g, expiry) = namespace(&f);
    let params = launch(&f, &g, expiry, "printf x >> once");
    let key = params["key"].clone();
    assert_eq!(f.request(&message("run.start", params.clone()))["ok"], true);
    assert_eq!(finished(&f, &g, &key)["run"]["completion"]["exit_code"], 0);
    let wrong = f.request(&message("run.list", json!({"generation":"0".repeat(32)})));
    assert_eq!(wrong["error"]["code"], "NAMESPACE_MISMATCH");
    std::fs::write(f.root.join("state/clock"), (expiry + 1).to_be_bytes())
        .unwrap_or_else(|e| panic!("clock: {e}"));
    let pruned = f.request(&message("run.prune", json!({"generation":g})));
    assert_eq!(pruned["data"]["removed"], json!([key]));
    let retry = f.request(&message("run.start", params));
    assert_eq!(retry["error"]["code"], "REQUEST_EXPIRED");
    assert_eq!(
        std::fs::read(f.root.join("once")).unwrap_or_else(|e| panic!("effects: {e}")),
        b"x"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn monitor_death_never_authorizes_a_duplicate_or_saved_pid_signal() {
    let f = Fixture::new();
    let (g, expiry) = namespace(&f);
    let params = launch(
        &f,
        &g,
        expiry,
        "printf '%s' \"$PPID\" > monitor; printf x >> once; i=0; while [ ! -f release ] && [ $i -lt 200 ]; do sleep 0.05; i=$((i+1)); done; printf done > child-finished",
    );
    let key = params["key"].clone();
    assert_eq!(f.request(&message("run.start", params.clone()))["ok"], true);
    let monitor =
        std::fs::read_to_string(f.root.join("monitor")).unwrap_or_else(|e| panic!("monitor: {e}"));
    let identity = PathBuf::from("/proc").join(&monitor).join("cwd");
    let expected = f
        .root
        .join("state/runs")
        .join(key.as_str().unwrap_or_else(|| panic!("key")));
    assert_eq!(
        std::fs::read_link(identity).unwrap_or_else(|e| panic!("identity: {e}")),
        expected
    );
    assert!(
        Command::new("kill")
            .args(["-KILL", &monitor])
            .status()
            .unwrap_or_else(|e| panic!("kill: {e}"))
            .success()
    );
    let lost = f.request(&message("run.poll", json!({"generation":g,"run_id":key})));
    assert_eq!(lost["data"]["run"]["status"], "lost");
    let retry = f.request(&message("run.start", params));
    assert_eq!(retry["data"]["run"]["status"], "lost");
    assert_eq!(
        std::fs::read(f.root.join("once")).unwrap_or_else(|e| panic!("effects: {e}")),
        b"x"
    );
    let refused = f.request(&message(
        "run.signal",
        json!({"generation":g,"run_id":key,"signal":"KILL"}),
    ));
    assert_eq!(refused["error"]["code"], "RUN_CONTROL_UNAVAILABLE");
    std::fs::write(f.root.join("release"), b"release").unwrap_or_else(|e| panic!("release: {e}"));
    for _ in 0..100 {
        if f.root.join("child-finished").exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("orphaned fixture child did not exit");
}
