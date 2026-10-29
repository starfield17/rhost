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
