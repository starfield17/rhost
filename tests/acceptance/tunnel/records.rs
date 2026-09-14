//! What the local records say afterwards: `list` reports what exists without
//! deleting it, `check` keeps its uncertainty, and `close` removes exactly one.

use super::*;

/// A stale record is evidence, not rhost's to discard: `list` reports it and
/// leaves it, and only `close` removes it.
#[test]
fn tunnel_list_is_empty_without_records_and_reports_stale_ones_without_deleting_them()
-> Result<(), String> {
    let empty = tunnel_master("tunnel-list-empty", "present")?;
    let (outcome, value) = envelope(&empty, &["tunnel", "list", "--json"])?;
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(operation(&value), "tunnel.list");
    assert_eq!(
        value["data"]["tunnels"],
        serde_json::Value::Array(Vec::new())
    );
    empty.assert_no_remote_tool("listing no tunnels")?;

    let harness = tunnel_master("tunnel-list-stale", "present")?;
    let (_, opened) = envelope(
        &harness,
        &[
            "tunnel",
            "open",
            "gpu",
            "--json",
            "--kind",
            "socks",
            "--listen",
            "localhost:1080",
        ],
    )?;
    let id = opened_id(&opened)?;
    assert!(
        opened["data"].get("destination").is_none(),
        "a socks forward has no destination: {opened}"
    );

    // The master is gone, and its socket went first: absence is the only
    // evidence rhost acts on, so this is the real shape of a dead master.
    let socket = master_socket(&harness)?;
    std::fs::remove_file(&socket).map_err(|error| fail("remove socket", error))?;
    let (outcome, value) = envelope(&harness, &["tunnel", "list", "--json"])?;
    assert_eq!(outcome.status, 0, "{value}");
    let rows = value["data"]["tunnels"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(rows.len(), 1, "{value}");
    assert_eq!(rows[0]["tunnel_id"], id.as_str());
    assert_eq!(rows[0]["status"], "stale");
    assert_eq!(rows[0]["kind"], "socks");
    assert_eq!(rows[0]["listen"], "localhost:1080");
    assert_eq!(
        state_files(&harness)?.len(),
        1,
        "reporting a stale record must not delete it"
    );

    let before = harness.calls_text();
    let (outcome, closed) = envelope(&harness, &["tunnel", "close", &id, "--json"])?;
    assert_eq!(outcome.status, 0, "{closed}");
    assert_eq!(closed["data"]["closed"], serde_json::Value::Bool(true));
    assert_eq!(
        before,
        harness.calls_text(),
        "there is no master left to ask about"
    );
    assert!(state_files(&harness)?.is_empty());
    Ok(())
}

/// A socket that is there but will not answer is not the same answer as a
/// socket that is gone: only absence is evidence that the master died.
#[test]
fn tunnel_check_uncertainty_is_retryable_and_preserves_the_record() -> Result<(), String> {
    let harness = tunnel_master("tunnel-uncertain", "present")?;
    let (_, opened) = envelope(
        &harness,
        &[
            "tunnel",
            "open",
            "gpu",
            "--json",
            "--kind",
            "local",
            "--listen",
            "localhost:8080",
            "--destination",
            "localhost:80",
        ],
    )?;
    let id = opened_id(&opened)?;
    harness.write("tunnel-mode", "uncertain", None)?;
    for args in [
        vec!["tunnel", "list", "--json"],
        vec!["tunnel", "close", id.as_str(), "--json"],
    ] {
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(outcome.status, 255, "{args:?} {value}");
        want_code(&value, "TUNNEL_FAILED").map_err(|error| format!("{args:?} {error}"))?;
        assert_eq!(
            value["error"]["retryable"],
            serde_json::Value::Bool(true),
            "{args:?}"
        );
    }
    assert_eq!(
        state_files(&harness)?.len(),
        1,
        "uncertainty is not evidence of death"
    );
    Ok(())
}

/// An id this machine has no record for is a different answer from a forward
/// that could not be stopped, and neither one runs a tool.
#[test]
fn tunnel_close_of_an_unknown_record_is_not_found_and_asks_no_master() -> Result<(), String> {
    let harness = tunnel_master("tunnel-close-unknown", "present")?;
    let unknown = format!("t_{}", "ab".repeat(16));
    for id in [unknown.as_str(), "not-a-tunnel-id"] {
        let (outcome, value) = envelope(&harness, &["tunnel", "close", id, "--json"])?;
        assert_eq!(outcome.status, 255, "{id} {value}");
        assert_eq!(operation(&value), "tunnel.close");
        want_code(&value, "TUNNEL_NOT_FOUND").map_err(|error| format!("{id} {error}"))?;
        assert_eq!(
            value["error"]["retryable"],
            serde_json::Value::Bool(false),
            "{id}"
        );
        assert_eq!(value["data"], serde_json::Value::Null);
    }
    harness.assert_no_remote_tool("unknown tunnel")
}
/// The human rendering carries the same facts as the envelope, including the
/// one thing a person has to keep: the id.
#[test]
fn tunnel_human_output_names_the_id_a_caller_has_to_keep() -> Result<(), String> {
    let harness = tunnel_master("tunnel-human", "present")?;
    let opened = run(
        &harness,
        &[
            "tunnel",
            "open",
            "gpu",
            "--kind",
            "local",
            "--listen",
            "localhost:8080",
            "--destination",
            "localhost:80",
        ],
    )?;
    assert_eq!(opened.status, 0, "{}", opened.stderr);
    let id = opened
        .stdout
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    assert!(id.starts_with("t_"), "{:?}", opened.stdout);
    assert!(
        opened.stderr.contains(&format!("rhost tunnel close {id}")),
        "the id has to arrive with its instruction: {:?}",
        opened.stderr
    );
    let listed = run(&harness, &["tunnel", "list"])?;
    assert!(listed.stdout.contains(&id), "{:?}", listed.stdout);
    let closed = run(&harness, &["tunnel", "close", &id])?;
    assert_eq!(closed.status, 0, "{}", closed.stderr);
    assert!(
        closed.stderr.contains(&format!("closed {id}")),
        "{:?}",
        closed.stderr
    );
    let empty = run(&harness, &["tunnel", "list"])?;
    assert!(empty.stderr.contains("no tunnels"), "{:?}", empty.stderr);
    Ok(())
}
