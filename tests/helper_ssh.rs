//! Exact-helper acceptance through real OpenSSH, with caller-supplied targets.
#![cfg(unix)]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
struct Host {
    binary: PathBuf,
    host: String,
    root: String,
    local: PathBuf,
}
fn required(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("set {name} explicitly"))
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn wait(mut child: Child) -> Result<std::process::Output, String> {
    let deadline = Instant::now() + Duration::from_secs(45);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("live helper invocation exceeded 45 seconds".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().map_err(|e| e.to_string())
}
impl Host {
    fn new() -> Result<Self, String> {
        let binary = std::fs::canonicalize(required("RHOST_BIN")?).map_err(|e| e.to_string())?;
        let host = required("RHOST_TEST_HOST")?;
        required("RHOST_HELPER_BIN")?;
        let output = Command::new("ssh")
            .args([
                "-T",
                "--",
                &host,
                "mktemp -d /tmp/rhost-helper-live-XXXXXXXX",
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("cannot create remote test directory over SSH".into());
        }
        let root = String::from_utf8(output.stdout)
            .map_err(|e| e.to_string())?
            .trim()
            .to_string();
        if !root.starts_with("/tmp/rhost-helper-live-") || root.contains(['\n', '\r']) {
            return Err("unsafe remote test directory".into());
        }
        let local = std::env::temp_dir().join(format!(
            "rhost-helper-live-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&local).map_err(|e| e.to_string())?;
        Ok(Self {
            binary,
            host,
            root,
            local,
        })
    }
    fn spawn(&self, operation: &str, params: Value) -> Result<Child, String> {
        let mut child = Command::new(&self.binary)
            .args(["request", "--from", "-"])
            .env("RHOST_HELPER_CACHE_DIR", format!("{}/helper", self.root))
            .env("RHOST_REMOTE_STATE", format!("{}/state", self.root))
            .env("RHOST_CACHE_DIR", self.local.join("transport"))
            .env("RHOST_STATE_DIR", self.local.join("state"))
            .env("RHOST_NO_SOURCE_BUILD", "1")
            .env("RHOST_NO_DOWNLOAD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        child
            .stdin
            .take()
            .ok_or("stdin")?
            .write_all(
                json!({"schema_version":3,"operation":operation,"host":self.host,"params":params})
                    .to_string()
                    .as_bytes(),
            )
            .map_err(|e| e.to_string())?;
        Ok(child)
    }
    fn ask(&self, operation: &str, params: Value) -> Result<Value, String> {
        self.decode(wait(self.spawn(operation, params)?)?)
    }
    fn decode(&self, output: std::process::Output) -> Result<Value, String> {
        let value: Value = serde_json::from_slice(&output.stdout).map_err(|e| {
            format!(
                "invalid response: {e}; stderr={:?}",
                String::from_utf8_lossy(&output.stderr)
            )
        })?;
        let schema: Value = serde_json::from_str(include_str!("../schemas/result-v3.schema.json"))
            .map_err(|e| e.to_string())?;
        assert!(
            jsonschema::validator_for(&schema)
                .map_err(|e| e.to_string())?
                .is_valid(&value),
            "{value}"
        );
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(output.status.success(), value["ok"] == true, "{value}");
        Ok(value)
    }
    fn ok(&self, operation: &str, params: Value) -> Result<Value, String> {
        let v = self.ask(operation, params)?;
        assert_eq!(v["ok"], true, "{v}");
        Ok(v["data"].clone())
    }
    fn prepare(&self) -> Result<(Value, u64), String> {
        let v = self.ok("helper.prepare", json!({}))?;
        let bytes = std::fs::read(required("RHOST_HELPER_BIN")?).map_err(|e| e.to_string())?;
        let hash: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(v["helper"]["binary_sha256"], hash);
        Ok((
            v["generation"].clone(),
            v["remote_time_unix"].as_u64().ok_or("remote time")? + 3600,
        ))
    }
    fn key(expiry: u64) -> String {
        format!("{expiry}-{:064x}", SEQUENCE.fetch_add(1, Ordering::Relaxed))
    }
    fn start(
        &self,
        generation: &Value,
        expiry: u64,
        key: &str,
        script: &str,
        wait_ms: u32,
        deadline_ms: u64,
    ) -> Value {
        json!({"generation":generation,"key":key,"expires_unix":expiry,"cwd":self.root,"program":{"argv":["/bin/sh","-c",script]},"environment":{"inherit":false,"values":{}},"wait_ms":wait_ms,"deadline_ms":deadline_ms})
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = Command::new("ssh")
            .args([
                "-T",
                "--",
                &self.host,
                &format!("rm -rf -- {}", quote(&self.root)),
            ])
            .stdin(Stdio::null())
            .output();
        let _ = std::fs::remove_dir_all(&self.local);
    }
}

#[test]
fn exact_image_readonly_metadata_and_reconnect() -> Result<(), String> {
    let h = Host::new()?;
    let absent = h.ask("metadata", json!({}))?;
    assert_eq!(absent["error"]["code"], "HELPER_PREPARATION_FAILED");
    let (g, _) = h.prepare()?;
    let a = h.ok("metadata", json!({}))?;
    let b = h.ok("metadata", json!({}))?;
    assert_eq!(a["generation"], g);
    assert_eq!(a["generation"], b["generation"]);
    assert_eq!(a["helper"]["binary_sha256"], b["helper"]["binary_sha256"]);
    assert_eq!(a["helper"]["os"], "linux");
    Ok(())
}

#[test]
fn concurrent_retries_reconnect_without_duplicate_effects() -> Result<(), String> {
    let h = Host::new()?;
    let (g, expiry) = h.prepare()?;
    let key = Host::key(expiry);
    let params = h.start(
        &g,
        expiry,
        &key,
        "printf x >> once; sleep 1; printf stdout; printf stderr >&2; exit 7",
        0,
        10000,
    );
    let children = (0..4)
        .map(|_| h.spawn("run.start", params.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    for child in children {
        let v = h.decode(wait(child)?)?;
        assert_eq!(v["ok"], true, "{v}");
    }
    let done = h.ok(
        "run.poll",
        json!({"generation":g,"run_id":key,"wait_ms":5000}),
    )?;
    assert_eq!(done["run"]["status"], "exited");
    assert_eq!(done["run"]["completion"]["exit_code"], 7);
    assert_eq!(done["stdout"]["content"], "stdout");
    assert_eq!(done["stderr"]["content"], "stderr");
    let repeated = h.ok(
        "run.poll",
        json!({"generation":g,"run_id":key,"stdout_since":3}),
    )?;
    assert_eq!(repeated["stdout"]["content"], "out");
    assert_eq!(
        h.ok("fs.read", json!({"path":format!("{}/once",h.root)}))?["content"],
        "x"
    );
    let mut changed = params;
    changed["program"] = json!({"argv":["/bin/false"]});
    assert_eq!(
        h.ask("run.start", changed)?["error"]["code"],
        "IDEMPOTENCY_CONFLICT"
    );
    Ok(())
}

#[test]
fn observer_cancellation_preserves_work_and_signals_are_explicit() -> Result<(), String> {
    let h = Host::new()?;
    let (g, expiry) = h.prepare()?;
    let key = Host::key(expiry);
    let started = h.ok(
        "run.start",
        h.start(
            &g,
            expiry,
            &key,
            "while [ ! -f release ]; do sleep 0.05; done",
            0,
            120000,
        ),
    )?;
    assert!(
        matches!(
            started["run"]["status"].as_str(),
            Some("accepted" | "running" | "starting")
        ),
        "{started}"
    );
    let observer = h.spawn(
        "run.poll",
        json!({"generation":g,"run_id":key,"wait_ms":30000}),
    )?;
    std::thread::sleep(Duration::from_millis(250));
    assert!(
        Command::new("kill")
            .args(["-INT", &observer.id().to_string()])
            .status()
            .map_err(|e| e.to_string())?
            .success()
    );
    let stopped = h.decode(wait(observer)?)?;
    assert_eq!(stopped["error"]["code"], "REMOTE_COMMAND_CANCELLED");
    assert_eq!(
        h.ok("run.poll", json!({"generation":g,"run_id":key}))?["run"]["status"],
        "running"
    );
    h.ok(
        "run.signal",
        json!({"generation":g,"run_id":key,"signal":"TERM"}),
    )?;
    let done = h.ok(
        "run.poll",
        json!({"generation":g,"run_id":key,"wait_ms":5000}),
    )?;
    assert_eq!(done["run"]["status"], "exited");
    assert_eq!(done["run"]["completion"]["signal"], 15);
    Ok(())
}

#[test]
fn deadlines_and_inherited_pipes_have_bounded_observation() -> Result<(), String> {
    let h = Host::new()?;
    let (g, expiry) = h.prepare()?;
    let deadline = Host::key(expiry);
    let done = h.ok(
        "run.start",
        h.start(&g, expiry, &deadline, "sleep 20", 5000, 200),
    )?;
    assert_eq!(done["run"]["status"], "exited");
    assert!(done["run"]["completion"]["signal"].as_u64().is_some());
    let key = Host::key(expiry);
    h.ok(
        "run.start",
        h.start(
            &g,
            expiry,
            &key,
            "(while [ ! -f release ]; do sleep 0.05; done) & printf leader",
            0,
            10000,
        ),
    )?;
    let done = h.ok(
        "run.poll",
        json!({"generation":g,"run_id":key,"wait_ms":5000}),
    )?;
    assert_eq!(done["run"]["completion"]["exit_code"], 0);
    assert_eq!(done["stdout"]["content"], "leader");
    // The descendant still holds the pipes: completion cannot depend on EOF.
    let final_page = h.ok(
        "run.poll",
        json!({"generation":g,"run_id":key,"wait_ms":5000}),
    )?;
    assert_eq!(final_page["stdout"]["incomplete"], true);
    h.ok(
        "fs.write",
        json!({"path":format!("{}/release",h.root),"content":"release"}),
    )?;
    Ok(())
}

#[test]
fn named_changesets_inspection_and_guarded_recovery_survive_clients() -> Result<(), String> {
    let h = Host::new()?;
    let (g, expiry) = h.prepare()?;
    let a = format!("{}/a.txt", h.root);
    let b = format!("{}/b.txt", h.root);
    let original = h.ok("fs.write", json!({"path":a,"content":"before\n"}))?;
    let id = Host::key(expiry);
    let p = json!({"generation":g,"key":id,"expires_unix":expiry,"changes":[{"op":"write","path":a,"content":"after\n","if_hash":original["sha256"]},{"op":"write","path":b,"content":"new\n"}]});
    assert_eq!(h.ok("fs.changeset.apply", p.clone())?["status"], "applied");
    assert_eq!(h.ok("fs.changeset.apply", p)?["status"], "applied");
    assert_eq!(
        h.ok("fs.changeset.query", json!({"generation":g,"id":id}))?["mutation"],
        "applied"
    );
    let search = h.ok(
        "fs.search",
        json!({"root":h.root,"glob":"*.txt","pattern":"after"}),
    )?;
    assert_eq!(search["items"].as_array().ok_or("search items")?.len(), 1);
    let read = h.ok("fs.read_many", json!({"paths":[a,b]}))?;
    assert_eq!(read["items"].as_array().ok_or("read items")?.len(), 2);
    assert_eq!(
        h.ok(
            "fs.changeset.recover",
            json!({"generation":g,"id":id,"action":"rollback"})
        )?["status"],
        "rolled_back"
    );
    assert_eq!(h.ok("fs.read", json!({"path":a}))?["content"], "before\n");
    let id = Host::key(expiry);
    let applied = h.ok("fs.changeset.apply", json!({"generation":g,"key":id,"expires_unix":expiry,"changes":[{"op":"write","path":a,"content":"installed\n","if_hash":original["sha256"]}]}))?;
    let hash = applied["files"][0]["after_hash"].clone();
    h.ok(
        "fs.write",
        json!({"path":a,"content":"external\n","if_hash":hash}),
    )?;
    let refused = h.ask(
        "fs.changeset.recover",
        json!({"generation":g,"id":id,"action":"rollback"}),
    )?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(h.ok("fs.read", json!({"path":a}))?["content"], "external\n");
    Ok(())
}
