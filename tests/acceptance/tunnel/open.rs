//! What `tunnel open` must refuse, and what it must prove before it records a
//! forward: a dedicated master that answers control-protocol requests, and a
//! socket the record's id can be resolved back to.

use super::*;

/// The grammar answers first: a tunnel command that cannot be understood is a
/// usage error, and nothing local or remote is touched to discover that.
#[test]
fn tunnel_grammar_refuses_what_it_cannot_run() -> Result<(), String> {
    let harness = tunnel_master("tunnel-usage", "present")?;
    for args in [
        vec!["tunnel", "open", "--json"],
        vec!["tunnel", "list", "gpu", "--json"],
        vec!["tunnel", "close", "--json"],
        vec!["tunnel", "reopen", "--json"],
        vec!["tunnel", "open", "gpu", "--json", "--kind"],
        vec![
            "tunnel",
            "open",
            "gpu",
            "--json",
            "--listen",
            "localhost:8080",
            "--destination",
            "localhost:80",
            "--tunnel",
            "extra",
        ],
    ] {
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(outcome.status, 255, "{args:?}");
        want_code(&value, "USAGE_ERROR").map_err(|error| format!("{args:?} {error}"))?;
        assert_eq!(operation(&value), "tunnel.usage", "{args:?}");
        assert_eq!(value["data"], serde_json::Value::Null);
    }
    harness.assert_no_remote_tool("tunnel grammar")
}

/// The two guards that are decided from local arguments alone, so a request
/// rhost would never build is refused before a directory is created or a master
/// is started.
#[test]
fn tunnel_open_refuses_an_impossible_forward_before_anything_else() -> Result<(), String> {
    let harness = tunnel_master("tunnel-validate", "present")?;
    for args in [
        vec![
            "--kind",
            "dynamic",
            "--listen",
            "localhost:8080",
            "--destination",
            "localhost:80",
        ],
        vec![
            "--kind",
            "local",
            "--listen",
            "localhost",
            "--destination",
            "localhost:80",
        ],
        vec!["--kind", "local", "--listen", "localhost:8080"],
        vec![
            "--kind",
            "local",
            "--listen",
            "localhost:0",
            "--destination",
            "localhost:80",
        ],
        vec!["--kind", "reverse", "--listen", "localhost:9000"],
        vec![
            "--kind",
            "socks",
            "--listen",
            "localhost:1080",
            "--destination",
            "localhost:80",
        ],
        vec![
            "--kind",
            "local",
            "--listen",
            "0.0.0.0:8080",
            "--destination",
            "localhost:80",
        ],
    ] {
        let mut full = vec!["tunnel", "open", "gpu", "--json"];
        full.extend(args.iter().copied());
        let (outcome, value) = envelope(&harness, &full)?;
        assert_eq!(outcome.status, 255, "{full:?}");
        assert_eq!(operation(&value), "tunnel.open", "{full:?}");
        want_code(&value, "CONFIG_INVALID").map_err(|error| format!("{full:?} {error}"))?;
        assert_eq!(
            value["error"]["retryable"],
            serde_json::Value::Bool(false),
            "{full:?} is a configuration answer, not a transport one"
        );
        assert_eq!(value["data"], serde_json::Value::Null);
    }
    assert!(
        state_files(&harness)?.is_empty(),
        "a refused forward must leave nothing to rediscover"
    );
    harness.assert_no_remote_tool("refused tunnels")
}

/// `open` returns an id, and that id has to mean the same thing to the next
/// process: the record is only written once the dedicated master answered.
#[test]
fn tunnel_open_starts_one_dedicated_master_and_records_the_id_it_reports() -> Result<(), String> {
    let harness = tunnel_master("tunnel-open", "present")?;
    let (outcome, value) = envelope(
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
    assert_eq!(outcome.status, 0, "{value}");
    assert_eq!(operation(&value), "tunnel.open");
    assert_eq!(value["data"]["host"], "gpu");
    assert_eq!(value["data"]["kind"], "local");
    assert_eq!(value["data"]["listen"], "localhost:8080");
    assert_eq!(value["data"]["destination"], "localhost:80");
    assert_eq!(
        value["data"]["status"], "alive",
        "an id is only reported once the master answered: {value}"
    );
    let id = opened_id(&value)?;
    let shape = Regex::new(r"^t_[0-9a-f]{32}$").map_err(|error| fail("regex", error))?;
    assert!(shape.is_match(&id), "unexpected tunnel id shape: {id}");

    // The master's own socket comes first, so OpenSSH keeps these values over
    // the ones rhost passes for every other command. Which directory that is
    // depends on how long the cache path is, so the argv is the evidence.
    let arguments = open_call(&harness)?;
    let socket = master_socket(&harness)?;
    assert!(
        socket.ends_with(&format!("/{id}")),
        "the tunnel's own socket is named after its id: {socket}"
    );
    let own = [
        "-o".to_string(),
        format!("ControlPath={socket}"),
        "-o".to_string(),
        "ControlMaster=yes".to_string(),
        "-o".to_string(),
        "ControlPersist=yes".to_string(),
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=15".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
    ];
    assert_eq!(
        &arguments[..own.len()],
        &own[..],
        "a tunnel must own its master: {arguments:?}"
    );
    let rest: Vec<&str> = arguments[own.len()..].iter().map(String::as_str).collect();
    assert!(
        rest.contains(&"ControlMaster=auto"),
        "the shared options still ride along, second: {rest:?}"
    );
    assert!(
        rest.iter().any(|argument| argument.ends_with("/%C")),
        "the shared template is passed second and loses to the tunnel's own path: {rest:?}"
    );
    assert_eq!(
        &rest[rest.len() - 5..],
        &["-fNT", "-L", "localhost:8080:localhost:80", "--", "gpu"],
        "one forward, one program, one target: {rest:?}"
    );

    let calls = harness.tool_calls("ssh");
    let opened = calls
        .iter()
        .position(|call| call.args.iter().any(|argument| argument == "-fNT"))
        .ok_or("no open call")?;
    let checked = calls
        .iter()
        .position(|call| call.args.iter().any(|argument| argument == "check"))
        .ok_or("the master was never asked whether it is up")?;
    assert!(
        opened < checked,
        "alive is an observation made after the start, never a hope: {}",
        harness.calls_text()
    );
    let check = &calls[checked].args;
    assert_eq!(
        check,
        &vec![
            "-S".to_string(),
            socket.clone(),
            "-O".to_string(),
            "check".to_string(),
            "--".to_string(),
            "gpu".to_string(),
        ],
        "the check goes to the tunnel's own socket, not the shared one"
    );

    // A later process rediscovers it, which is the whole point of the record.
    let (outcome, listed) = envelope(&harness, &["tunnel", "list", "--json"])?;
    assert_eq!(outcome.status, 0, "{listed}");
    assert_eq!(operation(&listed), "tunnel.list");
    let rows = listed["data"]["tunnels"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(rows.len(), 1, "{listed}");
    assert_eq!(rows[0]["tunnel_id"], id.as_str());
    assert_eq!(rows[0]["status"], "alive");
    assert_eq!(state_files(&harness)?.len(), 1, "one record, one tunnel");

    let (outcome, closed) = envelope(&harness, &["tunnel", "close", &id, "--json"])?;
    assert_eq!(outcome.status, 0, "{closed}");
    assert_eq!(operation(&closed), "tunnel.close");
    assert_eq!(closed["data"]["tunnel_id"], id.as_str());
    assert_eq!(closed["data"]["closed"], serde_json::Value::Bool(true));
    assert!(
        harness.calls_text().contains("-O exit"),
        "close asks that tunnel's master to exit: {}",
        harness.calls_text()
    );
    let (_, after) = envelope(&harness, &["tunnel", "list", "--json"])?;
    assert_eq!(
        after["data"]["tunnels"],
        serde_json::Value::Array(Vec::new()),
        "a closed tunnel must not still be listed: {after}"
    );
    assert!(
        state_files(&harness)?.is_empty(),
        "closing removes the record as well as the master"
    );
    Ok(())
}

/// A forward that was never proved is not a tunnel. Both halves matter: ssh's
/// own words reach the caller, and nothing is left behind for `list` to find.
#[test]
fn tunnel_open_that_cannot_prove_its_master_leaves_no_record() -> Result<(), String> {
    // `refused` is a bind the remote side rejected before any master existed;
    // `silent` is a start that left no socket behind; `uncertain` is a socket
    // that exists and will not answer, which is the one case with something to
    // take down again.
    for (mode, asked_to_exit) in [("refused", false), ("silent", false), ("uncertain", true)] {
        let harness = tunnel_master(&format!("tunnel-unproven-{mode}"), mode)?;
        let (outcome, value) = envelope(
            &harness,
            &[
                "tunnel",
                "open",
                "gpu",
                "--json",
                "--kind",
                "reverse",
                "--listen",
                "localhost:9000",
                "--destination",
                "localhost:3000",
            ],
        )?;
        assert_eq!(outcome.status, 255, "{mode}: {value}");
        assert_eq!(operation(&value), "tunnel.open");
        want_code(&value, "TUNNEL_FAILED").map_err(|error| format!("{mode} {error}"))?;
        assert_eq!(
            value["error"]["retryable"],
            serde_json::Value::Bool(true),
            "{mode}: a dropped master or an unreachable host is retryable"
        );
        assert_eq!(value["data"], serde_json::Value::Null);
        assert!(
            state_files(&harness)?.is_empty(),
            "{mode}: an unproved forward must not leave a record"
        );
        assert_eq!(
            harness.calls_text().contains("-O exit"),
            asked_to_exit,
            "{mode}: an unproved master is taken down only when there is a socket to address: {}",
            harness.calls_text()
        );
        if mode == "refused" {
            assert!(
                value["error"]["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("remote port forwarding failed"),
                "{mode}: OpenSSH's own complaint is the actionable part: {value}"
            );
        }
    }
    Ok(())
}
/// The exposure guard is the one local decision that changes who else can reach
/// the remote network, so it has to be asked for out loud.
#[test]
fn tunnel_open_only_leaves_loopback_when_it_is_asked_to() -> Result<(), String> {
    let harness = tunnel_master("tunnel-expose", "present")?;
    let (outcome, value) = envelope(
        &harness,
        &[
            "tunnel",
            "open",
            "gpu",
            "--json",
            "--kind",
            "local",
            "--listen",
            "0.0.0.0:8080",
            "--destination",
            "localhost:80",
            "--allow-exposure",
        ],
    )?;
    assert_eq!(outcome.status, 0, "{value}");
    let arguments = open_call(&harness)?;
    assert!(
        arguments
            .iter()
            .any(|argument| argument == "0.0.0.0:8080:localhost:80"),
        "the exposed bind is the one OpenSSH is asked for: {arguments:?}"
    );
    Ok(())
}
