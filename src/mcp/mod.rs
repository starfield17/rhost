//! Official SDK stdio delivery over the same capability requests as the CLI.
mod admission;
mod input;
mod schema;
mod selection;
use crate::transport::Client;
#[cfg(test)]
use admission::admit;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext};
pub use selection::Selection;
use serde_json::Value;
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
    admission: Arc<admission::Admission>,
}
struct Observation {
    cancelled: Arc<AtomicBool>,
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
            .with_instructions("Operate SSH-reachable hosts with structured files, recoverable noninteractive Runs and persistent tmux terminals. Use hosts/version without a target; doctor diagnoses shell dependencies, helper_prepare establishes native readiness. Read/search then hash-guard edits. Run start requires a retained generation and expiring key; wait/cancellation only observes. Use session send/read for REPLs; session exec timeout may interrupt, and canceling its observer does not revoke that submitted timeout. Inspect schema-v4 completion/mutation evidence before retry. OpenSSH owns authentication and host keys. Manage project directories and Git branches yourself.")
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
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let argument_bytes = arguments.to_string().len();
        let call = match crate::request::from_arguments(operation, arguments) {
            Ok(call) => call,
            Err(error) => {
                return Ok(CallToolResult::structured_error(crate::request::failure(
                    operation, None, error,
                ))
                .into());
            }
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = context.ct;
        let _observation = Observation {
            cancelled: cancelled.clone(),
        };
        let admission = self
            .admission
            .acquire(operation, argument_bytes, cancellation.cancelled())
            .await;
        let permit = match admission {
            Err(()) => {
                return Ok(CallToolResult::structured_error(crate::request::busy(
                    operation,
                    call.host.as_deref(),
                ))
                .into());
            }
            Ok(Some(permit)) => permit,
            Ok(None) => {
                return Ok(CallToolResult::structured_error(
                    crate::request::observation_cancelled(operation, call.host.as_deref()),
                )
                .into());
            }
        };
        let client = self.client.clone();
        let stopped = self.stopped.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            crate::request::execute_call(&client, call, &|| {
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
    let schema: Value = serde_json::from_str(include_str!("../../schemas/request-v4.schema.json"))
        .map_err(|e| e.to_string())?;
    let result: Value = serde_json::from_str(include_str!("../../schemas/result-v4.schema.json"))
        .map_err(|e| e.to_string())?;
    let operations = schema["properties"]["operation"]["enum"]
        .as_array()
        .ok_or("request operations missing")?;
    let mut tools = Vec::new();
    for value in operations {
        let operation = value.as_str().ok_or("invalid operation")?;
        let input = schema::input(&schema, operation)?;
        let output = schema::output(&result, operation)?;
        let description = match operation {
            "version" => {
                "Read local binary version, build provenance and schema generation. No host parameter."
            }
            "hosts" => {
                "Discover concrete aliases in the local OpenSSH client configuration. Best-effort completeness is explicit; no SSH target parameter."
            }
            "doctor" => {
                "Diagnose SSH authentication, shell command presence and state-directory writability. Inspect capabilities; this does not prepare the native helper."
            }
            "connection.status" => {
                "Observe the shared OpenSSH control master without creating or resetting it. Unknown master evidence is not proof of host reachability."
            }
            "session.create" => {
                "Create a persistent bash/tmux terminal. Retain its canonical identity; an unknown creation may still have effects and must be inspected before retry."
            }
            "session.list" => {
                "List persistent terminal records and current tmux liveness, including after another CLI/MCP process exits."
            }
            "session.exec" => {
                "Execute a command in an idle managed shell. Busy REPLs refuse; timeout may interrupt the pane. Observer cancellation does not revoke its submitted timeout."
            }
            "session.send" => {
                "Send text with optional Enter or one control key to an owned interactive terminal. No completion or deduplication guarantee; inspect before repeating uncertain input."
            }
            "session.read" => {
                "Read bounded terminal output by raw-byte since/tail cursor. Continue from next, never rendered text length; PTY stdout/stderr are merged."
            }
            "session.recover" => {
                "Explicitly interrupt the terminal foreground and verify that the managed shell is usable again. This can affect running interactive work."
            }
            "session.close" => {
                "Explicitly close the named persistent terminal. If confirmation is lost, inspect the session list before repeating."
            }
            "fs.read" => {
                "Read bounded UTF-8 lines and observe the entire file hash for a later CAS edit."
            }
            "fs.search" => {
                "Search remote files by POSIX glob and single-line literal text. Return structured line matches and observed file hashes; inspect incomplete and truncation indicators."
            }
            "fs.read_many" => {
                "Read bounded observations of multiple explicit paths, preserving input order and per-file hashes/errors. This is not a workspace snapshot."
            }
            "fs.changeset.apply" => {
                "Preflight named file CAS changes and journal individual replacements. Retain identity; no globally atomic visibility or automatic rollback."
            }
            "fs.changeset.query" => {
                "Inspect a named changeset's per-file states, mutation history and pending cleanup before choosing explicit recovery."
            }
            "fs.changeset.recover" => {
                "Explicitly resume or roll back a named changeset with guarded file versions, preserving later external edits."
            }
            "fs.changeset.prune" => {
                "Explicitly remove eligible expired terminal changeset journals, ending their recovery retention."
            }
            "run.list" => {
                "Discover retained Run registrations and lifecycle status in the expected namespace generation."
            }
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
            _ => "Perform one typed remote operation and return its schema-v4 envelope.",
        };
        let mut tool = Tool::new(
            format!("rhost_{}", operation.replace('.', "_")),
            description,
            input.as_object().ok_or("input schema missing")?.clone(),
        );
        tool.output_schema = Some(Arc::new(
            output.as_object().ok_or("result schema missing")?.clone(),
        ));
        tool.annotations = Some(selection::annotations(operation));
        tools.push((operation.to_string(), tool));
    }
    Ok(tools)
}
async fn interruption(interrupt: &crate::signals::Interrupt) {
    while !interrupt.requested() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
pub fn run(client: &Client, interrupt: &crate::signals::Interrupt, selection: &Selection) -> u8 {
    let result = (|| -> Result<(), String> {
        let tools = Arc::new(
            tools()?
                .into_iter()
                .filter(|(op, _)| selection.allows(op))
                .collect(),
        );
        let stopped = Arc::new(AtomicBool::new(false));
        let server = Server {
            client: Arc::new(client.clone()),
            tools,
            stopped: stopped.clone(),
            admission: Arc::new(admission::Admission::default()),
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
            use std::io::Write;
            let _ = writeln!(std::io::stderr().lock(), "rhost: INTERNAL: {error}");
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
