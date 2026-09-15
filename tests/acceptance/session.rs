//! Sessions: the grammar, the transport boundary, and the rule that a session
//! answer is never a success without evidence.
//!
//! End-to-end session behaviour needs a real pane — tmux, a pty and a shell — so
//! it is verified against a real host (docs/RUST_MIGRATION.md). What is pinned
//! here is everything that has to be decided locally, and the answers that must
//! stay failures.
use crate::support::{Harness, envelope, operation, run, shell_quote, want_code};

/// The whole surface the parser accepts, and nothing in it reaches ssh.
#[test]
fn session_grammar_refuses_what_it_cannot_run() -> Result<(), String> {
    let harness = Harness::open("session-usage")?;
    harness.stub("ssh", "exit 99")?;
    for args in [
        vec!["session", "create", "--json"],
        vec!["session", "list", "--json"],
        vec!["session", "exec", "gpu", "--json"],
        vec!["session", "exec", "gpu", "work", "--json"],
        vec!["session", "exec", "gpu", "work", "--json", "--command", ""],
        vec![
            "session",
            "exec",
            "gpu",
            "work",
            "--json",
            "--command",
            "true",
            "--command",
            "ls",
        ],
        vec!["session", "exec", "gpu", "work", "--json", "--", "true"],
        vec!["session", "read", "gpu", "--json"],
        vec!["session", "read", "gpu", "work", "--json", "--since", "-1"],
        vec!["session", "recover", "gpu", "--json"],
        vec!["session", "close", "gpu", "--json"],
        vec![
            "session",
            "create",
            "gpu",
            "--json",
            "--timeout",
            "nonsense",
        ],
        vec!["session", "list", "gpu", "extra", "--json"],
        vec!["session", "list", "gpu", "--json", "--cwd", "/tmp"],
        vec!["session", "reopen", "gpu", "--json"],
    ] {
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(outcome.status, 255, "{args:?}");
        assert_eq!(operation(&value), "session.usage", "{args:?}");
        want_code(&value, "USAGE_ERROR").map_err(|error| format!("{args:?} {error}"))?;
        assert_eq!(value["data"], serde_json::Value::Null);
    }
    harness.assert_no_remote_tool("session grammar")
}

/// Local arguments that name something rhost cannot honour are configuration
/// answers, decided before a helper is built or a host is contacted.
#[test]
fn session_configuration_mistakes_never_reach_a_host() -> Result<(), String> {
    let harness = Harness::open("session-config")?;
    harness.stub("ssh", "exit 99")?;
    for args in [
        vec!["session", "create", "gpu", "--json", "--shell", "zsh"],
        vec!["session", "create", "gpu", "--json", "--name", "bad name"],
        vec!["session", "create", "gpu", "--json", "--name", "x/y"],
        vec!["session", "send", "gpu", "work", "--json"],
        vec![
            "session", "send", "gpu", "work", "--json", "--data", "x", "--key", "C-c",
        ],
        vec![
            "session", "send", "gpu", "work", "--json", "--key", "C-c", "--enter",
        ],
        vec!["session", "send", "gpu", "work", "--json", "--enter"],
        vec![
            "session", "send", "gpu", "work", "--json", "--data", "x", "--key", "F13",
        ],
        vec!["session", "send", "gpu", "work", "--json", "--data", ""],
    ] {
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(outcome.status, 255, "{args:?}");
        want_code(&value, "CONFIG_INVALID").map_err(|error| format!("{args:?} {error}"))?;
        assert_eq!(
            value["error"]["retryable"],
            serde_json::Value::Bool(false),
            "{args:?}"
        );
    }
    harness.assert_no_remote_tool("session configuration")
}

/// Attaching needs a terminal, so the wire contract has no envelope it could
/// honestly carry: the refusal names the real operation instead of pretending to
/// have attached.
#[test]
fn session_attach_is_refused_under_its_own_operation() -> Result<(), String> {
    let harness = Harness::open("session-attach")?;
    harness.stub("ssh", "exit 99")?;
    let (outcome, value) = envelope(&harness, &["session", "attach", "gpu", "work", "--json"])?;
    assert_eq!(outcome.status, 255);
    assert_eq!(operation(&value), "session.attach");
    want_code(&value, "USAGE_ERROR")?;
    assert_eq!(value["error"]["retryable"], serde_json::Value::Bool(false));
    assert_eq!(value["data"], serde_json::Value::Null);
    let human = run(&harness, &["session", "attach", "gpu", "work"])?;
    assert_eq!(human.status, 255);
    assert!(human.stderr.contains("USAGE_ERROR"), "{:?}", human.stderr);
    harness.assert_no_remote_tool("session attach")
}

/// One session call is one helper: the command string crosses the SSH boundary
/// as a single program after `--`, exactly as `exec` does, so nothing about the
/// caller's operands is re-parsed by a local shell.
#[test]
fn a_session_call_submits_exactly_one_helper_program() -> Result<(), String> {
    let harness = Harness::open("session-boundary")?;
    harness.stub("ssh", "exit 0")?;
    let (outcome, value) = envelope(&harness, &["session", "list", "gpu", "--json"])?;
    // The stub answers nothing, so this can only be the "no completion evidence"
    // answer: a session call must never turn silence into success.
    assert_eq!(outcome.status, 255, "{value}");
    want_code(&value, "REMOTE_EXECUTION_UNKNOWN")?;
    let calls = harness.tool_calls("ssh");
    assert_eq!(calls.len(), 1, "{}", harness.calls_text());
    let arguments = &calls[0].args;
    let boundary = arguments
        .iter()
        .position(|argument| argument == "--")
        .ok_or("no -- boundary in the submission")?;
    assert_eq!(arguments.get(boundary + 1).map(String::as_str), Some("gpu"));
    assert_eq!(
        arguments.len() - boundary - 1,
        2,
        "target plus one helper program, not a rebuilt argv: {arguments:?}"
    );
    assert!(
        arguments.iter().any(|argument| argument == "-T"),
        "a session helper is not interactive: {arguments:?}"
    );
    Ok(())
}

/// A transport failure is reported as itself. Its cause is the connection, not
/// the session, and it must not be dressed up as a session refusal.
#[test]
fn session_transport_failures_keep_their_own_codes() -> Result<(), String> {
    for (diagnostic, code, retryable) in [
        ("Permission denied (publickey).", "SSH_AUTH_FAILED", false),
        (
            "ssh: connect to host example-host port 22: Connection refused",
            "SSH_UNREACHABLE",
            true,
        ),
    ] {
        let harness = Harness::open(&format!("session-{}", code.to_lowercase()))?;
        harness.stub(
            "ssh",
            &format!("printf '%s\\n' {} >&2\nexit 255", shell_quote(diagnostic)),
        )?;
        for args in [
            vec!["session", "list", "gpu", "--json"],
            vec![
                "session",
                "exec",
                "gpu",
                "work",
                "--json",
                "--command",
                "true",
            ],
        ] {
            let (outcome, value) = envelope(&harness, &args)?;
            assert_eq!(outcome.status, 255, "{code}: {value}");
            want_code(&value, code).map_err(|error| format!("{code} {error}"))?;
            assert_eq!(
                value["error"]["retryable"],
                serde_json::Value::Bool(retryable),
                "{code}"
            );
            assert_eq!(value["data"], serde_json::Value::Null);
        }
    }
    Ok(())
}

/// A create whose answer was lost still reports the candidate identity and says
/// the outcome is unknown, so the caller can look the session up (SESSION-010).
#[test]
fn a_create_without_completion_evidence_reports_the_candidate_and_unknown() -> Result<(), String> {
    let harness = Harness::open("session-create-unknown")?;
    // The submission is accepted and then the channel goes silent: a timeout or
    // a disconnect after the remote may already have created the session.
    harness.stub("ssh", "exit 0")?;
    let (outcome, value) = envelope(&harness, &["session", "create", "gpu", "--json"])?;
    assert_eq!(outcome.status, 255, "{value}");
    assert_eq!(operation(&value), "session.create");
    want_code(&value, "REMOTE_EXECUTION_UNKNOWN")?;
    assert_eq!(value["error"]["retryable"], serde_json::Value::Bool(false));
    let id = value["data"]["session_id"]
        .as_str()
        .ok_or("create failure must name a candidate session id")?;
    assert!(id.starts_with("s_"), "{value}");
    assert_eq!(value["data"]["creation_status"], "unknown");
    // The name defaults to the id when the caller gives none.
    assert_eq!(value["data"]["session_ref"], id);
    // The human line names a candidate and points at `session list`.
    let human = run(&harness, &["session", "create", "gpu"])?;
    assert_eq!(human.status, 255);
    assert!(
        human.stderr.contains("candidate session s_") && human.stderr.contains("session list"),
        "{:?}",
        human.stderr
    );
    Ok(())
}

/// An explicit remote refusal is a definite negative: no session was kept, so
/// the creation status is `not_created`.
#[test]
fn a_refused_create_reports_the_candidate_and_not_created() -> Result<(), String> {
    for (helper_code, want) in [
        ("nameinuse", "CONFIG_INVALID"),
        ("notmux", "REMOTE_DEPENDENCY_MISSING"),
        ("noflock", "REMOTE_DEPENDENCY_MISSING"),
    ] {
        let harness = Harness::open(&format!("session-create-{helper_code}"))?;
        // The helper prints its refusal as its own stdout and exits zero.
        harness.stub(
            "ssh",
            &format!("printf 'RHOST_ERR={helper_code}\\n'\nexit 0"),
        )?;
        let (outcome, value) = envelope(&harness, &["session", "create", "gpu", "--json"])?;
        assert_eq!(outcome.status, 255, "{helper_code}: {value}");
        want_code(&value, want).map_err(|error| format!("{helper_code} {error}"))?;
        assert!(
            value["data"]["session_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("s_")),
            "{helper_code}: {value}"
        );
        assert_eq!(
            value["data"]["creation_status"], "not_created",
            "{helper_code}: a refusal keeps no session"
        );
    }
    Ok(())
}

/// A create that failed before the command was submitted (bad target,
/// authentication, unreachable host) proves nothing was created, so it stays the
/// ordinary null-data failure rather than inventing a candidate session.
#[test]
fn a_create_that_never_reached_the_host_has_no_candidate_session() -> Result<(), String> {
    for (diagnostic, code) in [
        ("Permission denied (publickey).", "SSH_AUTH_FAILED"),
        (
            "ssh: connect to host example-host port 22: Connection refused",
            "SSH_UNREACHABLE",
        ),
    ] {
        let harness = Harness::open(&format!("session-create-{}", code.to_lowercase()))?;
        harness.stub(
            "ssh",
            &format!("printf '%s\\n' {} >&2\nexit 255", shell_quote(diagnostic)),
        )?;
        let (outcome, value) = envelope(&harness, &["session", "create", "gpu", "--json"])?;
        assert_eq!(outcome.status, 255, "{code}: {value}");
        want_code(&value, code)?;
        assert_eq!(
            value["data"],
            serde_json::Value::Null,
            "{code}: a pre-submission failure has no candidate session"
        );
    }
    Ok(())
}

/// A local mistake never reached the host, so there is no candidate to report.
#[test]
fn a_local_create_mistake_has_no_candidate_session() -> Result<(), String> {
    let harness = Harness::open("session-create-local")?;
    harness.stub("ssh", "exit 99")?;
    let (outcome, value) = envelope(
        &harness,
        &["session", "create", "gpu", "--json", "--shell", "zsh"],
    )?;
    assert_eq!(outcome.status, 255);
    want_code(&value, "CONFIG_INVALID")?;
    assert_eq!(
        value["data"],
        serde_json::Value::Null,
        "a local mistake has no candidate identity"
    );
    harness.assert_no_remote_tool("session create local mistake")
}
