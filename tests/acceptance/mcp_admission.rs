//! A saturated observer lane must leave independent control admission available.
use super::mcp::{Mcp, occupied};
use crate::support::Harness;
use serde_json::json;

#[test]
fn control_can_signal_a_run_while_all_observation_slots_are_occupied() -> Result<(), String> {
    let harness = Harness::open("mcp-control-admission")?;
    crate::hosts::local_host(&harness)?;
    let state = harness.path("remote-state").to_string_lossy().into_owned();
    let harness = harness.env("RHOST_REMOTE_STATE", &state);
    let mut m = Mcp::open(&harness)?;
    let (generation, key) = occupied(&mut m, &harness)?;
    let response = m.call(
        40,
        "rhost_run_signal",
        json!({"host":"gpu","params":{"generation":generation,"run_id":key,"signal":"TERM"}}),
    )?;
    assert_eq!(response["data"]["delivered"], true, "{response}");
    m.close()
}

#[test]
fn invalid_input_has_structured_not_submitted_recovery() -> Result<(), String> {
    let h = Harness::open("recovery-validation")?;
    let (_, answer) = crate::support::envelope_with_stdin(&h, &["request", "--from", "-"], b"[]")?;
    assert_eq!(
        answer["error"]["recovery"],
        json!({"stage":"validation","submission":"not_submitted","action":"correct_request"})
    );
    h.assert_no_remote_tool("invalid request")
}

#[test]
fn tool_groups_filter_listing_and_invocation_and_annotate_effects() -> Result<(), String> {
    let h = Harness::open("mcp-tool-groups")?;
    let mut m = Mcp::open_with_args(&h, &["mcp", "--tools", "runs"])?;
    m.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))?;
    let listed = m.answer(2)?;
    let tools = listed["result"]["tools"].as_array().ok_or("tools")?;
    assert!(tools.iter().any(|t| t["name"] == "rhost_run_signal"));
    assert!(!tools.iter().any(|t| t["name"] == "rhost_fs_write"));
    let version = tools
        .iter()
        .find(|t| t["name"] == "rhost_version")
        .ok_or("version")?;
    assert_eq!(version["annotations"]["readOnlyHint"], true);
    let signal = tools
        .iter()
        .find(|t| t["name"] == "rhost_run_signal")
        .ok_or("signal")?;
    assert_eq!(signal["annotations"]["destructiveHint"], true);
    assert_eq!(signal["annotations"]["idempotentHint"], false);
    m.start(
        3,
        "rhost_fs_write",
        json!({"host":"gpu","params":{"path":"/must-not-be-written","content":"hidden"}}),
    )?;
    assert!(m.answer(3)?["error"].is_object());
    m.close()?;
    h.assert_no_remote_tool("filtered tool")
}
