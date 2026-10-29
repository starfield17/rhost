//! A bounded real stdio client; remote targets still come from Live's environment.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
pub(super) struct Mcp {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Result<Value, String>>,
    sequence: u64,
}
impl Mcp {
    pub fn open(mut command: Command) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().ok_or("MCP stdin")?;
        let stdout = child.stdout.take().ok_or("MCP stdout")?;
        let (sender, output) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender
                    .send(
                        line.map_err(|e| e.to_string())
                            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string())),
                    )
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input: Some(input),
            output,
            sequence: 1,
        };
        client.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"live-verification","version":"0"}}}))?;
        assert!(client.answer(1)?["result"]["capabilities"]["tools"].is_object());
        client.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        Ok(client)
    }
    fn send(&mut self, value: Value) -> Result<(), String> {
        let input = self.input.as_mut().ok_or("MCP input closed")?;
        writeln!(input, "{value}")
            .and_then(|()| input.flush())
            .map_err(|e| e.to_string())
    }
    fn answer(&self, id: u64) -> Result<Value, String> {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let value = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|e| e.to_string())??;
            if value["id"] == id {
                return Ok(value);
            }
        }
    }
    pub fn start(
        &mut self,
        operation: &str,
        host: Option<&str>,
        params: Value,
    ) -> Result<u64, String> {
        self.sequence += 1;
        let id = self.sequence;
        let mut arguments = json!({"params":params});
        if let Some(host) = host {
            arguments["host"] = host.into();
        }
        self.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":format!("rhost_{}",operation.replace('.',"_")),"arguments":arguments}}))?;
        Ok(id)
    }
    pub fn call(
        &mut self,
        operation: &str,
        host: Option<&str>,
        params: Value,
    ) -> Result<Value, String> {
        let id = self.start(operation, host, params)?;
        let response = self.answer(id)?;
        let value = response["result"]["structuredContent"].clone();
        let schema: Value =
            serde_json::from_str(include_str!("../../schemas/result-v3.schema.json"))
                .map_err(|e| e.to_string())?;
        assert!(
            jsonschema::validator_for(&schema)
                .map_err(|e| e.to_string())?
                .is_valid(&value),
            "{response}"
        );
        Ok(value)
    }
    pub fn cancel(&mut self, id: u64) -> Result<(), String> {
        self.send(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}),
        )
    }
    pub fn close(mut self) -> Result<(), String> {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                assert!(status.success());
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("MCP did not exit after EOF".into());
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
