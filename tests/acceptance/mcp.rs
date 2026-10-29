//! Exercise the public stdio delivery form, including concurrent cancellation.
use crate::support::{Harness, fail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
pub(super) struct Mcp {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Result<Value, String>>,
}
impl Mcp {
    pub(super) fn open(harness: &Harness) -> Result<Self, String> {
        let mut child = harness
            .command(&["mcp"])?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| fail("start MCP", e))?;
        let input = child.stdin.take().ok_or("MCP input missing")?;
        let stdout = child.stdout.take().ok_or("MCP output missing")?;
        let (sender, output) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value = line
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()));
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        let mut m = Self {
            child,
            input: Some(input),
            output,
        };
        m.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"acceptance","version":"0"}}}))?;
        let initialized = m.answer(1)?;
        assert!(
            initialized["result"]["capabilities"]["tools"].is_object(),
            "{initialized}"
        );
        m.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        Ok(m)
    }
    pub(super) fn send(&mut self, value: Value) -> Result<(), String> {
        let input = self.input.as_mut().ok_or("MCP input closed")?;
        writeln!(input, "{value}")
            .and_then(|_| input.flush())
            .map_err(|e| fail("write MCP", e))
    }
    pub(super) fn answer(&self, id: u64) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let value = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|e| fail("MCP response", e))??;
            if value["id"] == id {
                return Ok(value);
            }
        }
    }
    pub(super) fn start(&mut self, id: u64, name: &str, args: Value) -> Result<(), String> {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":args}}))
    }
    pub(super) fn call(&mut self, id: u64, name: &str, args: Value) -> Result<Value, String> {
        self.start(id, name, args)?;
        let response = self.answer(id)?;
        let envelope = response["result"]["structuredContent"].clone();
        let schema: Value =
            serde_json::from_str(include_str!("../../schemas/result-v3.schema.json"))
                .map_err(|e| e.to_string())?;
        let validator = jsonschema::validator_for(&schema).map_err(|e| e.to_string())?;
        assert!(validator.is_valid(&envelope), "{response}");
        Ok(envelope)
    }
    pub(super) fn close(&mut self) -> Result<(), String> {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                assert!(status.success(), "{status}");
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("MCP did not exit after stdin EOF".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_lists_typed_tools_and_returns_the_same_file_envelope() -> Result<(), String> {
    let harness = Harness::open("mcp-files")?;
    crate::hosts::local_host(&harness)?;
    let mut m = Mcp::open(&harness)?;
    m.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))?;
    let listed = m.answer(2)?;
    let tools = listed["result"]["tools"]
        .as_array()
        .ok_or("tools missing")?;
    assert!(tools.iter().any(|t| t["name"] == "rhost_run_start"));
    assert!(tools.iter().any(|t| t["name"] == "rhost_fs_patch"));
    let path = harness
        .path("file 'quotes'; $dollar")
        .to_string_lossy()
        .into_owned();
    let written = m.call(
        3,
        "rhost_fs_write",
        json!({"host":"gpu","params":{"path":path,"content":"mcp\n"}}),
    )?;
    assert_eq!(written["data"]["mutation"], "applied");
    let read = m.call(
        4,
        "rhost_fs_read",
        json!({"host":"gpu","params":{"path":path}}),
    )?;
    assert_eq!(read["data"]["content"], "mcp\n");
    let refused = m.call(
        5,
        "rhost_fs_read",
        json!({"host":"gpu","params":{"path":path,"max_bytes":0}}),
    )?;
    assert_eq!(refused["error"]["code"], "CONFIG_INVALID");
    m.close()
}

#[test]
fn mcp_cancellation_is_scoped_and_does_not_cancel_the_run() -> Result<(), String> {
    let harness = Harness::open("mcp-cancel")?;
    crate::hosts::local_host(&harness)?;
    let state = harness.path("remote-state").to_string_lossy().into_owned();
    let harness = harness.env("RHOST_REMOTE_STATE", &state);
    let mut m = Mcp::open(&harness)?;
    let prepared = m.call(2, "rhost_helper_prepare", json!({"host":"gpu","params":{}}))?;
    let generation = prepared["data"]["generation"].clone();
    let expiry = prepared["data"]["remote_time_unix"]
        .as_u64()
        .ok_or("remote clock missing")?
        + 3600;
    let key = format!("{expiry}-{}", "c".repeat(64));
    let started=m.call(3,"rhost_run_start",json!({"host":"gpu","params":{"generation":generation,"key":key,"expires_unix":expiry,"cwd":"/","program":{"argv":["/bin/sleep","30"]},"environment":{"inherit":false,"values":{}},"wait_ms":0}}))?;
    assert!(matches!(
        started["data"]["run"]["status"].as_str(),
        Some("accepted" | "starting" | "running")
    ));
    let params = json!({"generation":generation,"run_id":key,"wait_ms":30000});
    m.start(4, "rhost_run_poll", json!({"host":"gpu","params":params}))?;
    let listed = m.call(
        5,
        "rhost_run_list",
        json!({"host":"gpu","params":{"generation":generation}}),
    )?;
    assert_eq!(listed["ok"], true, "{listed}");
    m.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":4,"reason":"stop observation"}}))?;
    let polled = m.call(
        6,
        "rhost_run_poll",
        json!({"host":"gpu","params":{"generation":generation,"run_id":key,"wait_ms":0}}),
    )?;
    assert!(
        matches!(
            polled["data"]["run"]["status"].as_str(),
            Some("accepted" | "starting" | "running")
        ),
        "{polled}"
    );
    let signaled = m.call(
        7,
        "rhost_run_signal",
        json!({"host":"gpu","params":{"generation":generation,"run_id":key,"signal":"TERM"}}),
    )?;
    assert_eq!(signaled["data"]["delivered"], true, "{signaled}");
    m.close()
}

pub(super) fn occupied(m: &mut Mcp, harness: &Harness) -> Result<(Value, String), String> {
    let prepared = m.call(2, "rhost_helper_prepare", json!({"host":"gpu","params":{}}))?;
    let generation = prepared["data"]["generation"].clone();
    let expiry = prepared["data"]["remote_time_unix"]
        .as_u64()
        .ok_or("clock missing")?
        + 3600;
    let key = format!("{expiry}-{}", "d".repeat(64));
    let started=m.call(3,"rhost_run_start",json!({"host":"gpu","params":{"generation":generation,"key":key,"expires_unix":expiry,"cwd":"/","program":{"argv":["/bin/sleep","30"]},"environment":{"inherit":false,"values":{}}}}))?;
    assert_eq!(started["ok"], true, "{started}");
    let submissions = harness.path("submitted-polls");
    std::fs::create_dir(&submissions).map_err(|e| e.to_string())?;
    let observed = crate::hosts::LOCAL_HOST.replace(
        "exec /bin/bash",
        &format!(
            "case \"$remote\" in *RHOST_FRAME_3_*) : > {}/\"$$\";; esac\nexec /bin/bash",
            crate::support::shell_quote(&submissions.to_string_lossy())
        ),
    );
    harness.stub("ssh", &observed)?;
    let submitted = || {
        std::fs::read_dir(&submissions)
            .map(|entries| entries.count())
            .unwrap_or(0)
    };
    for id in 10..26 {
        m.start(
            id,
            "rhost_run_poll",
            json!({"host":"gpu","params":{"generation":generation,"run_id":key,"wait_ms":30000}}),
        )?;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while submitted() < 16 {
        if Instant::now() >= deadline {
            return Err("16 observations did not reach SSH".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok((generation, key))
}

#[test]
fn mcp_eof_stops_active_observers_and_never_submits_queued_work() -> Result<(), String> {
    let harness = Harness::open("mcp-eof-active")?;
    crate::hosts::local_host(&harness)?;
    let state = harness.path("remote-state").to_string_lossy().into_owned();
    let harness = harness.env("RHOST_REMOTE_STATE", &state);
    let mut m = Mcp::open(&harness)?;
    let (generation, key) = occupied(&mut m, &harness)?;
    let target = harness.path("must-not-be-written");
    m.start(
        30,
        "rhost_fs_write",
        json!({"host":"gpu","params":{"path":target,"content":"queued"}}),
    )?;
    m.close()?;
    assert!(!target.exists(), "EOF must cancel queued submission");
    let (_, polled) = crate::support::envelope(
        &harness,
        &[
            "run",
            "poll",
            "gpu",
            &key,
            "--generation",
            generation.as_str().ok_or("generation missing")?,
            "--json",
        ],
    )?;
    assert_eq!(polled["data"]["run"]["status"], "running", "{polled}");
    let (_, signaled) = crate::support::envelope(
        &harness,
        &[
            "run",
            "signal",
            "gpu",
            &key,
            "--generation",
            generation.as_str().ok_or("generation missing")?,
            "--sig",
            "TERM",
            "--json",
        ],
    )?;
    assert_eq!(signaled["data"]["delivered"], true, "{signaled}");
    Ok(())
}

#[test]
fn a_cancelled_queued_call_remains_unsubmitted_when_capacity_returns() -> Result<(), String> {
    let harness = Harness::open("mcp-queued-cancel")?;
    crate::hosts::local_host(&harness)?;
    let state = harness.path("remote-state").to_string_lossy().into_owned();
    let harness = harness.env("RHOST_REMOTE_STATE", &state);
    let mut m = Mcp::open(&harness)?;
    let (generation, key) = occupied(&mut m, &harness)?;
    let target = harness.path("must-not-be-written");
    m.start(
        30,
        "rhost_fs_write",
        json!({"host":"gpu","params":{"path":target,"content":"queued"}}),
    )?;
    m.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":30}}))?;
    // The SDK deliberately suppresses responses after a cancellation notice.
    m.send(json!({"jsonrpc":"2.0","id":40,"method":"tools/list"}))?;
    assert!(m.answer(40)?["result"]["tools"].is_array());
    assert!(!target.exists());
    for id in 10..26 {
        m.send(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}),
        )?;
    }
    let result = m.call(
        31,
        "rhost_run_signal",
        json!({"host":"gpu","params":{"generation":generation,"run_id":key,"signal":"TERM"}}),
    )?;
    assert_eq!(result["data"]["delivered"], true, "{result}");
    m.close()?;
    assert!(
        !target.exists(),
        "cancelled queued mutation must stay unsubmitted"
    );
    Ok(())
}
