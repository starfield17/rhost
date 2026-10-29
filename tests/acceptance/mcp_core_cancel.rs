//! Cancellation applies to one observer and never launches queued core work.
use crate::mcp::{Mcp, occupied};
use crate::support::{Harness, shell_quote};
use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn cancelling_a_core_probe_stops_later_submissions_and_preserves_other_calls() -> Result<(), String>
{
    let h = Harness::open("mcp-core-cancel")?;
    let marker = h.path("probe-ready");
    h.stub(
        "ssh",
        &format!(
            ": > {}\nexec /bin/sleep 30",
            shell_quote(&marker.to_string_lossy())
        ),
    )?;
    let mut m = Mcp::open(&h)?;
    m.start(3, "rhost_doctor", json!({"host":"gpu","params":{}}))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() {
        if Instant::now() >= deadline {
            return Err("probe did not reach SSH".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let unaffected = m.call(4, "rhost_version", json!({"params":{}}))?;
    assert_eq!(unaffected["ok"], true);
    m.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}))?;
    m.close()?;
    assert_eq!(
        h.calls().len(),
        1,
        "cancellation must prevent later probe and cleanup submissions"
    );
    Ok(())
}

#[test]
fn queued_core_mutations_remain_unsubmitted_after_cancel_or_eof() -> Result<(), String> {
    for eof in [false, true] {
        let h = Harness::open(if eof {
            "mcp-core-eof"
        } else {
            "mcp-core-queued"
        })?;
        crate::hosts::local_host(&h)?;
        let state = h.path("remote-state").to_string_lossy().into_owned();
        let h = h.env("RHOST_REMOTE_STATE", &state);
        let mut m = Mcp::open(&h)?;
        let (generation, key) = occupied(&mut m, &h)?;
        let calls = h.calls().len();
        m.start(
            30,
            "rhost_session_send",
            json!({"host":"gpu","params":{"session":"debug","data":"must-not-send","enter":true}}),
        )?;
        if !eof {
            m.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":30}}))?;
            m.send(json!({"jsonrpc":"2.0","id":40,"method":"tools/list"}))?;
            assert!(m.answer(40)?["result"]["tools"].is_array());
            for id in 10..26 {
                m.send(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}))?;
            }
            assert_eq!(
                m.call(41, "rhost_version", json!({"params":{}}))?["ok"],
                true
            );
        }
        m.close()?;
        assert_eq!(
            h.calls().len(),
            calls,
            "queued session input must never reach SSH"
        );
        let (_, result) = crate::support::envelope(
            &h,
            &[
                "run",
                "signal",
                "gpu",
                &key,
                "--generation",
                generation.as_str().ok_or("generation")?,
                "--sig",
                "TERM",
                "--json",
            ],
        )?;
        assert_eq!(result["data"]["delivered"], true);
    }
    Ok(())
}
