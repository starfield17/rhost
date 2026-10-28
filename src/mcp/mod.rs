//! Official SDK stdio delivery over the same capability requests as the CLI.
mod input;
use crate::transport::Client;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Clone)]
struct Server {
    client: Arc<Client>,
    tools: Arc<Vec<(String, Tool)>>,
    stopped: Arc<AtomicBool>,
    slots: Arc<tokio::sync::Semaphore>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    host: String,
    params: Value,
}
struct Observation {
    cancelled: Arc<AtomicBool>,
}
async fn admit(
    slots: Arc<tokio::sync::Semaphore>,
    cancelled: impl std::future::Future<Output = ()>,
) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, tokio::sync::AcquireError> {
    tokio::select! {biased;_=cancelled=>Ok(None),result=slots.acquire_owned()=>result.map(Some)}
}
impl Drop for Observation {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("rhost",env!("CARGO_PKG_VERSION")))
            .with_instructions("OpenSSH owns authentication and host keys. Run start requires a caller-owned expiring key. Wait cancellation stops observation; signal is explicit. File mutations report evidence in the schema-v3 envelope. Manage project directories and Git branches yourself.")
    }
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(
            self.tools.iter().map(|(_, t)| t.clone()).collect(),
        ))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some((operation, _)) = self.tools.iter().find(|(_, t)| t.name == request.name) else {
            return Err(ErrorData::method_not_found::<CallToolRequestMethod>());
        };
        let arguments: Arguments =
            match serde_json::from_value(Value::Object(request.arguments.unwrap_or_default())) {
                Ok(value) => value,
                Err(error) => {
                    return Ok(CallToolResult::structured_error(crate::request::invalid(
                        error.to_string(),
                    ))
                    .into());
                }
            };
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = context.ct;
        let _observation = Observation {
            cancelled: cancelled.clone(),
        };
        let Some(permit) = admit(self.slots.clone(), cancellation.cancelled())
            .await
            .map_err(|_| ErrorData::internal_error("server closed", None))?
        else {
            return Ok(
                CallToolResult::structured_error(crate::request::observation_cancelled(
                    operation,
                    &arguments.host,
                ))
                .into(),
            );
        };
        let client = self.client.clone();
        let stopped = self.stopped.clone();
        let raw=json!({"schema_version":3,"operation":operation,"host":arguments.host,"params":arguments.params}).to_string();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            crate::request::execute(&client, &raw, &|| {
                cancellation.is_cancelled()
                    || cancelled.load(Ordering::SeqCst)
                    || stopped.load(Ordering::SeqCst)
            })
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        Ok(if result["ok"] == true {
            CallToolResult::structured(result)
        } else {
            CallToolResult::structured_error(result)
        }
        .into())
    }
}
fn tools() -> Result<Vec<(String, Tool)>, String> {
    let schema: Value = serde_json::from_str(include_str!("../../schemas/request-v3.schema.json"))
        .map_err(|e| e.to_string())?;
    let result: Value = serde_json::from_str(include_str!("../../schemas/result-v3.schema.json"))
        .map_err(|e| e.to_string())?;
    let operations = schema["properties"]["operation"]["enum"]
        .as_array()
        .ok_or("request operations missing")?;
    let mut tools = Vec::new();
    for value in operations {
        let operation = value.as_str().ok_or("invalid operation")?;
        let branches = schema["allOf"]
            .as_array()
            .ok_or("request branches missing")?;
        let params = branches
            .iter()
            .find(|branch| {
                let condition = &branch["if"]["properties"]["operation"];
                condition["const"] == operation
                    || condition["enum"]
                        .as_array()
                        .is_some_and(|ops| ops.iter().any(|v| v == operation))
            })
            .map(|branch| branch["then"]["properties"]["params"].clone())
            .ok_or("operation parameters missing")?;
        let input = json!({"type":"object","additionalProperties":false,"required":["host","params"],"properties":{"host":schema["properties"]["host"],"params":params},"$defs":schema["$defs"]});
        let description = match operation {
            "run.start" => {
                "Register or rediscover a detached Run using the same immutable key and payload. wait_ms only observes; deadline_ms is enforced remotely."
            }
            "run.poll" => {
                "Observe a Run and bounded separate byte streams. Repeated retained cursors are safe; transport loss does not establish completion."
            }
            "run.signal" => {
                "Explicitly signal the live owning Run monitor. No saved PID is used when ownership cannot be proved."
            }
            "run.prune" => {
                "Explicitly prune expired completed Run records. Active Runs are retained."
            }
            "helper.prepare" => {
                "Prepare a matching native helper and remote namespace without installing system packages."
            }
            "metadata" => "Read an existing helper namespace without creating one.",
            "fs.write" | "fs.patch" => {
                "Apply a hash-guarded file edit. Inspect mutation evidence even on failure."
            }
            _ => "Perform one typed remote operation and return its schema-v3 envelope.",
        };
        let mut tool = Tool::new(
            format!("rhost_{}", operation.replace('.', "_")),
            description,
            input.as_object().ok_or("input schema missing")?.clone(),
        );
        tool.output_schema = Some(Arc::new(
            result.as_object().ok_or("result schema missing")?.clone(),
        ));
        tools.push((operation.to_string(), tool));
    }
    Ok(tools)
}
async fn interruption(interrupt: &crate::signals::Interrupt) {
    while !interrupt.requested() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
pub fn run(client: &Client, interrupt: &crate::signals::Interrupt) -> u8 {
    let result = (|| -> Result<(), String> {
        let tools = Arc::new(tools()?);
        let stopped = Arc::new(AtomicBool::new(false));
        let server = Server {
            client: Arc::new(client.clone()),
            tools,
            stopped: stopped.clone(),
            slots: Arc::new(tokio::sync::Semaphore::new(16)),
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let result=runtime.block_on(async {
            let (reader,writer)=rmcp::transport::stdio_pipes();
            let service=tokio::select!{result=server.serve((input::Input::new(reader,stopped.clone()),writer))=>result.map_err(|e|e.to_string())?,_=interruption(interrupt)=>return Ok(())};
            let token=service.cancellation_token();
            tokio::select!{result=service.waiting()=>result.map(|_|()).map_err(|e|e.to_string()),_=interruption(interrupt)=>{token.cancel();Ok(())}}
        });
        stopped.store(true, Ordering::SeqCst);
        runtime.shutdown_timeout(Duration::from_secs(2));
        result
    })();
    match result {
        Ok(()) => 0,
        Err(error) => {
            crate::cli::warn(&format!("rhost: INTERNAL: {error}"));
            255
        }
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn ready_cancellation_wins_over_ready_capacity() {
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        for _ in 0..32 {
            let admitted = super::admit(slots.clone(), std::future::ready(()))
                .await
                .unwrap_or_else(|e| panic!("admit: {e}"));
            assert!(admitted.is_none());
            assert_eq!(slots.available_permits(), 1);
        }
    }
    #[tokio::test]
    async fn cancelling_admission_does_not_need_a_free_permit() {
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let held = slots
            .clone()
            .acquire_owned()
            .await
            .unwrap_or_else(|e| panic!("hold permit: {e}"));
        let (send, receive) = tokio::sync::oneshot::channel::<()>();
        send.send(()).unwrap_or_else(|_| panic!("cancel admission"));
        let admitted = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            super::admit(slots.clone(), async {
                let _ = receive.await;
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("cancelled call still waited for capacity"))
        .unwrap_or_else(|e| panic!("admit: {e}"));
        assert!(admitted.is_none());
        assert_eq!(slots.available_permits(), 0);
        drop(held);
        assert_eq!(slots.available_permits(), 1);
    }
}
