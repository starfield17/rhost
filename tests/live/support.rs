use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SCHEMA: &str = include_str!("../../schemas/result-v2.schema.json");
const CALL_DEADLINE: Duration = Duration::from_secs(180);
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub(crate) struct Live {
    binary: PathBuf,
    host: String,
    root: PathBuf,
}

pub(crate) struct Answer {
    pub(crate) output: Output,
    pub(crate) value: serde_json::Value,
}

impl Live {
    pub(crate) fn new(case: &str) -> Result<Self, String> {
        let binary = required("RHOST_BIN")?;
        let binary = std::fs::canonicalize(&binary)
            .map_err(|error| format!("RHOST_BIN {binary:?}: {error}"))?;
        let host = required("RHOST_TEST_HOST")?;
        let root = std::env::temp_dir().join(unique(&format!("rhost-live-{case}")));
        for dir in ["cache", "state", "tmp"] {
            std::fs::create_dir_all(root.join(dir))
                .map_err(|error| format!("prepare live scratch: {error}"))?;
        }
        let live = Self { binary, host, root };
        let version = live.json(&["version", "--json"])?;
        assert_eq!(version.value["schema_version"], 2);
        Ok(live)
    }

    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub(crate) fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .args(args)
            .env("RHOST_CACHE_DIR", self.root.join("cache"))
            .env("RHOST_STATE_DIR", self.root.join("state"))
            .env("TMPDIR", self.root.join("tmp"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    pub(crate) fn command_with_env(&self, args: &[&str], key: &str, value: &Path) -> Command {
        let mut command = self.command(args);
        command.env(key, value);
        command
    }

    pub(crate) fn run(&self, args: &[&str]) -> Result<Output, String> {
        let child = self
            .command(args)
            .spawn()
            .map_err(|error| format!("spawn rhost {args:?}: {error}"))?;
        wait_bounded(child, CALL_DEADLINE)
    }

    pub(crate) fn json(&self, args: &[&str]) -> Result<Answer, String> {
        decode(self.run(args)?, args)
    }

    pub(crate) fn json_with_env(
        &self,
        args: &[&str],
        key: &str,
        value: &Path,
    ) -> Result<Answer, String> {
        let child = self
            .command_with_env(args, key, value)
            .spawn()
            .map_err(|error| format!("spawn rhost {args:?}: {error}"))?;
        decode(wait_bounded(child, CALL_DEADLINE)?, args)
    }

    pub(crate) fn ok(&self, args: &[&str]) -> Result<Answer, String> {
        let answer = self.json(args)?;
        if answer.value["ok"] != true || !answer.value["error"].is_null() {
            return Err(format!("{args:?} failed: {}", answer.value));
        }
        Ok(answer)
    }

    pub(crate) fn error(&self, code: &str, args: &[&str]) -> Result<Answer, String> {
        let answer = self.json(args)?;
        if answer.value["ok"] != false || answer.value["error"]["code"] != code {
            return Err(format!("{args:?}: want {code}, got {}", answer.value));
        }
        Ok(answer)
    }

    pub(crate) fn exec(&self, program: &str) -> Result<Answer, String> {
        self.ok(&["--json", "exec", &self.host, "--command", program])
    }

    pub(crate) fn remote_dir(&self) -> Result<String, String> {
        let answer = self.exec("mktemp -d /tmp/rhost-rust-live-XXXXXX")?;
        let dir = text(&answer.value, "/data/output/stdout/content")?
            .trim()
            .to_string();
        if !dir.starts_with("/tmp/rhost-rust-live-") {
            return Err(format!("unsafe remote scratch returned: {dir:?}"));
        }
        Ok(dir)
    }

    pub(crate) fn cleanup_remote(&self, dir: &str) -> Result<(), String> {
        if !dir.starts_with("/tmp/rhost-rust-live-") {
            return Err(format!("refuse cleanup outside live prefix: {dir}"));
        }
        let command = format!("rm -rf -- {}", shell_quote(dir));
        let answer = self.exec(&command)?;
        if number(&answer.value, "/data/execution/exit_code")? != 0 {
            return Err(format!("remote cleanup failed: {}", answer.value));
        }
        Ok(())
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn decode(output: Output, args: &[&str]) -> Result<Answer, String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).map_err(|error| {
        format!(
            "{args:?}: stdout is not one JSON document: {error}; stdout={stdout:?}; stderr={:?}",
            String::from_utf8_lossy(&output.stderr)
        )
    })?;
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).map_err(|error| format!("load schema: {error}"))?;
    let validator =
        jsonschema::validator_for(&schema).map_err(|error| format!("compile schema: {error}"))?;
    if !validator.is_valid(&value) {
        return Err(format!("{args:?}: schema rejected {value}"));
    }
    if value["ok"].as_bool().unwrap_or(false) != value["error"].is_null() {
        return Err(format!("{args:?}: ok/error disagree: {value}"));
    }
    Ok(Answer { output, value })
}

pub(crate) fn text<'a>(value: &'a serde_json::Value, pointer: &str) -> Result<&'a str, String> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("{pointer} is not known text: {value}"))
}

pub(crate) fn number(value: &serde_json::Value, pointer: &str) -> Result<i64, String> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| format!("{pointer} is not a known integer: {value}"))
}

pub(crate) fn boolean(value: &serde_json::Value, pointer: &str) -> Result<bool, String> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| format!("{pointer} is not a known boolean: {value}"))
}

pub(crate) fn wait_bounded(mut child: Child, deadline: Duration) -> Result<Output, String> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("collect child output: {error}"));
            }
            Ok(None) if started.elapsed() < deadline => sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("rhost did not finish within {deadline:?}"));
            }
            Err(error) => return Err(format!("wait for rhost: {error}")),
        }
    }
}

pub(crate) fn poll(mut check: impl FnMut() -> Result<bool, String>) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if check()? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("condition was not observed within 30s".to_string());
        }
        sleep(Duration::from_millis(200));
    }
}

pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(crate) fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "{prefix}-{}-{nanos}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|error| format!("write {}: {error}", path.display()))
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("set {name} explicitly to run native live tests"))
}
