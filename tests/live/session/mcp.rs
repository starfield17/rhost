use crate::mcp_client::Mcp;
use crate::support::{Live, poll, text, unique};
use serde_json::json;

#[test]
fn mcp_session_state_busy_io_timeout_and_close_use_the_existing_contract() -> Result<(), String> {
    let live = Live::new("mcp-session")?;
    let host = live.host().to_string();
    let name = unique("rlive-mcp");
    let mut m = Mcp::open(live.command(&["mcp"]))?;
    let created = m.call(
        "session.create",
        Some(&host),
        json!({"name":name,"cwd":"/tmp"}),
    )?;
    assert_eq!(created["ok"], true, "{created}");
    let id = text(&created, "/data/session_id")?.to_string();
    let result: Result<(), String> = (|| {
        let executed=m.call("session.exec",Some(&host),json!({"session":id,"command":"export RHOST_LIVE_VALUE=retained; printf '中文-finished'; false"}))?;
        assert_eq!(executed["data"]["execution"]["exit_code"], 1, "{executed}");
        assert!(text(&executed, "/data/output/content")?.contains("中文-finished"));
        assert_eq!(m.call("session.list", Some(&host), json!({}))?["ok"], true);
        assert_eq!(
            m.call(
                "session.send",
                Some(&host),
                json!({"session":id,"data":"cat","enter":true})
            )?["ok"],
            true
        );
        poll(|| {
            let refused = m.call(
                "session.exec",
                Some(&host),
                json!({"session":id,"command":"true"}),
            )?;
            Ok(refused["error"]["code"] == "SESSION_BUSY")
        })?;
        assert_eq!(
            m.call(
                "session.send",
                Some(&host),
                json!({"session":id,"data":"mcp-input-observed","enter":true})
            )?["ok"],
            true
        );
        poll(|| {
            Ok(m.call(
                "session.read",
                Some(&host),
                json!({"session":id,"since":0,"max_bytes":4096}),
            )?["data"]["content"]
                .as_str()
                .unwrap_or("")
                .contains("mcp-input-observed"))
        })?;
        let page = m.call(
            "session.read",
            Some(&host),
            json!({"session":id,"since":0,"max_bytes":64}),
        )?;
        let next = page["data"]["next"].as_u64().ok_or("cursor")?;
        assert!(next > 0);
        let resumed = m.call(
            "session.read",
            Some(&host),
            json!({"session":id,"since":next,"max_bytes":64}),
        )?;
        assert_eq!(resumed["data"]["from"], next);
        assert_eq!(
            m.call("session.recover", Some(&host), json!({"session":id}))?["data"]["session_preserved"],
            true
        );
        let timeout = m.call(
            "session.exec",
            Some(&host),
            json!({"session":id,"command":"sleep 30","timeout_ms":1000}),
        )?;
        assert_eq!(
            timeout["error"]["code"], "REMOTE_COMMAND_TIMEOUT",
            "{timeout}"
        );
        let _ = m.call("session.recover", Some(&host), json!({"session":id}))?;
        let value = m.call(
            "session.exec",
            Some(&host),
            json!({"session":id,"command":"printf '%s' \"$RHOST_LIVE_VALUE\""}),
        )?;
        assert_eq!(value["data"]["output"]["content"], "retained");
        assert_eq!(
            m.call("session.close", Some(&host), json!({"session":id}))?["ok"],
            true
        );
        Ok(())
    })();
    let _ = live.json(&["session", "close", &host, &id, "--json"]);
    result?;
    m.close()
}

#[test]
fn mcp_cancel_and_eof_leave_a_running_pane_available_to_a_new_process() -> Result<(), String> {
    for cancelled in [false, true] {
        cancellation_case(cancelled)?;
    }
    Ok(())
}
fn cancellation_case(cancelled: bool) -> Result<(), String> {
    let live = Live::new("mcp-session-cancel")?;
    let host = live.host().to_string();
    let name = unique("rlive-mcp-cancel");
    let created = live.ok(&["session", "create", &host, "--name", &name, "--json"])?;
    let id = text(&created.value, "/data/session_id")?.to_string();
    let result: Result<(), String> = (|| {
        let mut m = Mcp::open(live.command(&["mcp"]))?;
        let request=m.start("session.exec",Some(&host),json!({"session":id,"command":"sh -c 'trap \"printf unintended-interrupt\" INT; printf observer-ready; while :; do sleep 1; done'","timeout_ms":60000}))?;
        poll(|| {
            Ok(m.call(
                "session.read",
                Some(&host),
                json!({"session":id,"max_bytes":4096}),
            )?["data"]["content"]
                .as_str()
                .unwrap_or("")
                .contains("observer-ready"))
        })?;
        if cancelled {
            m.cancel(request)?;
        }
        if cancelled {
            assert_eq!(m.call("version", None, json!({}))?["ok"], true);
        }
        m.close()?;
        let mut reconnected = Mcp::open(live.command(&["mcp"]))?;
        let read = reconnected.call(
            "session.read",
            Some(&host),
            json!({"session":id,"max_bytes":4096}),
        )?;
        let content = text(&read, "/data/content")?;
        assert!(content.contains("observer-ready"));
        assert!(
            !content.contains("unintended-interrupt"),
            "cancellation must not send C-c"
        );
        live.error(
            "SESSION_BUSY",
            &["session", "exec", &host, &id, "--command", "true", "--json"],
        )?;
        reconnected.call("session.recover", Some(&host), json!({"session":id}))?;
        reconnected.close()
    })();
    let cleanup = live.ok(&["session", "close", &host, &id, "--json"]);
    result?;
    cleanup?;
    Ok(())
}
