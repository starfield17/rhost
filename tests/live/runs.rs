use super::{
    mcp_client::Mcp,
    support::{Live, decode, text, wait_bounded},
};
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

struct Namespace<'a> {
    live: &'a Live,
    state: String,
    generation: String,
    expiry: u64,
}

impl<'a> Namespace<'a> {
    fn new(live: &'a Live) -> Result<Self, String> {
        let mut namespace = Self {
            live,
            state: live.remote_dir()?,
            generation: String::new(),
            expiry: 0,
        };
        let prepared = namespace.call(&["helper", "prepare", live.host(), "--json"], None)?;
        assert_eq!(prepared["ok"], true, "{prepared}");
        namespace.generation = text(&prepared, "/data/generation")?.to_owned();
        namespace.expiry = prepared["data"]["remote_time_unix"]
            .as_u64()
            .ok_or("remote clock")?
            + 3600;
        Ok(namespace)
    }

    fn command(&self, args: &[&str]) -> std::process::Command {
        self.live
            .command_with_env(args, "RHOST_REMOTE_STATE", Path::new(&self.state))
    }

    fn call(&self, args: &[&str], delegation: Option<&Path>) -> Result<Value, String> {
        let mut command = self.command(args);
        if let Some(path) = delegation {
            command.env("RHOST_CGROUP_ROOT", path);
        }
        let process = command.spawn().map_err(|e| e.to_string())?;
        Ok(decode(wait_bounded(process, Duration::from_secs(120))?, args)?.value)
    }

    fn run(&self, leaf: &str, id: &str, options: &[&str]) -> Result<Value, String> {
        let mut args = vec![
            "run",
            leaf,
            self.live.host(),
            id,
            "--generation",
            &self.generation,
            "--json",
        ];
        args.extend_from_slice(options);
        let value = self.call(&args, None)?;
        assert_eq!(value["ok"], true, "{value}");
        Ok(value)
    }
}

impl Drop for Namespace<'_> {
    fn drop(&mut self) {
        let _ = self.live.cleanup_remote(&self.state);
    }
}

#[test]
fn rh02_follow_resumes_binary_data_after_mcp_disconnect() -> Result<(), String> {
    let live = Live::new("run-follow")?;
    let ns = Namespace::new(&live)?;
    let id = format!("{}-{}", ns.expiry, "a".repeat(64));
    let mut mcp = Mcp::open(ns.command(&["mcp"]))?;
    let started = mcp.call("run.start", Some(live.host()), json!({
        "generation": ns.generation, "key": id, "expires_unix": ns.expiry, "cwd": "/tmp",
        "program":{"argv":["/bin/sh","-c","printf '\\377abc'; printf 'error' >&2; sleep 1; printf def; exit 7"]},
        "environment":{"inherit":false,"values":{}},"deadline_ms":15000
    }))?;
    assert_eq!(started["ok"], true, "{started}");
    let page = mcp.call(
        "run.watch",
        Some(live.host()),
        json!({
            "generation":ns.generation, "run_id":id, "stdout_since":0, "stderr_since":0,
            "max_bytes":4, "wait_ms":5000
        }),
    )?;
    assert_eq!(page["ok"], true, "{page}");
    assert_eq!(page["data"]["stdout"]["from"], 0);
    let stdout = page["data"]["stdout"]["next"]
        .as_u64()
        .ok_or("stdout cursor")?;
    let stderr = page["data"]["stderr"]["next"]
        .as_u64()
        .ok_or("stderr cursor")?;
    assert!(matches!(stdout, 0 | 4) && matches!(stderr, 0 | 4));
    assert!(stdout == 4 || stderr == 4);
    assert_eq!(
        page["data"]["stdout"]["content_base64"],
        if stdout == 4 { "/2FiYw==" } else { "" }
    );
    assert_eq!(
        page["data"]["stderr"]["content_base64"],
        if stderr == 4 { "ZXJybw==" } else { "" }
    );
    mcp.close()?;
    let resumed = ns.run(
        "watch",
        &id,
        &[
            "--stdout-since",
            &stdout.to_string(),
            "--stderr-since",
            &stderr.to_string(),
            "--wait",
            "10s",
            "--max-bytes",
            "16",
        ],
    )?;
    assert_eq!(resumed["data"]["stdout"]["next"], 7);
    assert_eq!(
        resumed["data"]["stdout"]["content_base64"],
        if stdout == 4 { "ZGVm" } else { "/2FiY2RlZg==" }
    );
    assert_eq!(
        resumed["data"]["stderr"]["content_base64"],
        if stderr == 4 { "cg==" } else { "ZXJyb3I=" }
    );
    assert_eq!(resumed["data"]["run"]["completion"]["exit_code"], 7);
    assert_eq!(resumed["data"]["stdout"]["open"], false);
    Ok(())
}

#[test]
fn cgroup_kills_a_detached_setsid_descendant_after_leader_exit() -> Result<(), String> {
    let live = Live::new("run-cgroup")?;
    let ns = Namespace::new(&live)?;
    let id = format!("{}-{}", ns.expiry, "b".repeat(64));
    let delegation = std::env::var_os("RHOST_TEST_CGROUP_ROOT");
    let root = delegation
        .as_deref()
        .map_or(Path::new(&ns.state), Path::new);
    let started = ns.call(
        &[
            "run",
            "start",
            live.host(),
            "--generation",
            &ns.generation,
            "--key",
            &id,
            "--containment",
            "cgroup",
            "--cwd",
            "/tmp",
            "--wait",
            "5s",
            "--deadline",
            "30s",
            "--command",
            "setsid -f /bin/sleep 60 </dev/null >/dev/null 2>&1",
            "--json",
        ],
        Some(root),
    )?;
    if delegation.is_none() {
        assert_eq!(
            started["error"]["code"], "CONTAINMENT_UNAVAILABLE",
            "{started}"
        );
        assert_eq!(started["error"]["recovery"]["submission"], "not_submitted");
        return Ok(());
    }
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["data"]["run"]["containment"], "cgroup");
    assert_eq!(started["data"]["run"]["completion"]["exit_code"], 0);
    assert_eq!(started["data"]["run"]["containment_populated"], true);
    let signaled = ns.run("signal", &id, &["--sig", "KILL"])?;
    assert_eq!(signaled["data"]["delivered"], true);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let polled = ns.run("poll", &id, &[])?;
        assert_eq!(polled["data"]["run"]["completion"]["exit_code"], 0);
        if polled["data"]["run"]["containment_populated"] == false {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("Run cgroup stayed populated: {polled}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}
