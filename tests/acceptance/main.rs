//! Black-box acceptance for the Rust candidate.
//!
//! These tests drive the built binary the way an agent does: argv in, envelope
//! out. They are the candidate's own acceptance evidence, independent of the
//! frozen Go unit tests, and every document the binary prints on stdout is
//! validated against `schemas/result-v2.schema.json` before any field of it is
//! inspected.
//!
//! The stubs stop at the OpenSSH boundary. They deliberately neither emit nor
//! parse completion markers: CONTRACT.md keeps the wrapper encoding and marker
//! spelling free, so a hermetic stub that fabricated a marker would test one
//! implementation's private protocol instead of the evidence rule. Everything
//! that needs real completion evidence — a nonzero remote status, a timeout with
//! a known exit, an inherited background pipe — is verified against a real host
//! instead (see `docs/RUST_MIGRATION.md`).
//!
//! One test crate, cut by capability: `support` is the harness, `exec` the
//! foreground command, `files` the transfers, `edit` the remote editing surface
//! `remote_fs` the embedded helper’s direct conformance oracle, and
//! `transport` the connection and probe questions.
#![cfg(unix)]

mod audit;
mod edit;
mod exec;
mod files;
mod hosts;
mod remote_fs;
mod remote_fs_support;
mod session;
mod support;
mod transport;
mod tunnel;

use support::{Harness, envelope, operation, run, want_code};

#[test]
fn version_reports_the_binary_and_the_schema_independently() -> Result<(), String> {
    let harness = Harness::open("version")?;
    let (outcome, value) = envelope(&harness, &["version", "--json"])?;
    assert_eq!(outcome.status, 0);
    assert_eq!(outcome.stderr, "", "no diagnostics may join the document");
    assert_eq!(operation(&value), "version");
    assert_eq!(value["schema_version"], 2);
    assert_eq!(
        value
            .pointer("/data/version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
        env!("CARGO_PKG_VERSION"),
        "the version must come from the package manifest"
    );
    assert_eq!(
        value.pointer("/data/schema_version"),
        Some(&serde_json::Value::from(2)),
        "the data must agree with the envelope"
    );
    harness.assert_no_remote_tool("version")
}

#[test]
fn human_version_output_has_no_counterpart_only_where_the_schema_allows_none() -> Result<(), String>
{
    let harness = Harness::open("version-human")?;
    let outcome = run(&harness, &["version"])?;
    assert_eq!(outcome.status, 0);
    assert!(
        outcome.stdout.contains(env!("CARGO_PKG_VERSION")),
        "human output must name the version: {:?}",
        outcome.stdout
    );
    assert!(
        !outcome.stdout.contains("schema_version"),
        "human prose must not fake a JSON field name"
    );
    // v2 retired the implementation-specific field, so it may appear in neither
    // rendering (CONTRACT.md "Explicit v2 differences").
    assert!(!outcome.stdout.contains("go version"));
    Ok(())
}

#[test]
fn an_empty_config_discovers_no_hosts_and_says_so() -> Result<(), String> {
    let harness = Harness::open("hosts-empty")?;
    let (outcome, value) = envelope(&harness, &["hosts", "--json"])?;
    assert_eq!(outcome.status, 0);
    assert_eq!(operation(&value), "hosts");
    assert_eq!(
        value
            .pointer("/data/hosts")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(0),
        "a wildcard must not become a host"
    );
    assert_eq!(value["data"]["config_found"], serde_json::Value::Bool(true));
    assert_eq!(value["data"]["complete"], serde_json::Value::Bool(true));
    harness.assert_no_remote_tool("hosts")
}

#[test]
fn discovered_aliases_are_concrete_sorted_and_matched_by_both_renderings() -> Result<(), String> {
    let harness = Harness::open("hosts-list")?;
    harness.write(
        "home/.ssh/config",
        "Host zeta\n  HostName example-host\nHost alpha\n  User nobody\nHost * 10.* !excluded\n  Port 22\nHost beta gamma\n  ProxyJump alpha\n",
        None,
    )?;
    let (_, value) = envelope(&harness, &["hosts", "--json"])?;
    let aliases: Vec<String> = value["data"]["hosts"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("alias").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    assert_eq!(aliases, vec!["alpha", "beta", "gamma", "zeta"]);
    // The human rendering must show the same set, one alias per line (§6).
    let outcome = run(&harness, &["hosts"])?;
    let lines: Vec<&str> = outcome.stdout.lines().collect();
    assert_eq!(lines, vec!["alpha", "beta", "gamma", "zeta"]);
    harness.assert_no_remote_tool("hosts")
}

#[test]
fn invalid_invocations_are_usage_errors_carried_in_the_envelope() -> Result<(), String> {
    let harness = Harness::open("usage")?;
    for args in [
        vec!["--json"],
        vec!["exec", "--json"],
        vec!["job", "--json"],
        vec!["--json", "--not-a-flag"],
        vec!["exec", "gpu", "--json"],
        vec!["exec", "gpu", "extra", "--json", "--command", "date"],
        vec!["exec", "gpu", "--json", "--command", ""],
        vec![
            "exec",
            "gpu",
            "--json",
            "--command",
            "date",
            "--command",
            "date",
        ],
        vec!["exec", "--json", "--", "gpu", "date"],
        vec![
            "exec",
            "gpu",
            "--json",
            "--command",
            "date",
            "--timeout",
            "nonsense",
        ],
        vec!["doctor", "--json"],
        vec!["connection", "status", "--json"],
        vec!["connection", "bogus", "gpu", "--json"],
        vec!["hosts", "extra", "--json"],
    ] {
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(
            outcome.status, 255,
            "{args:?} must not look like a remote status"
        );
        want_code(&value, "USAGE_ERROR").map_err(|error| format!("{args:?} {error}"))?;
    }
    // `--stream` without `--json` is refused in the rendering that has no
    // envelope to carry it.
    let human = run(&harness, &["exec", "gpu", "--stream", "--command", "date"])?;
    assert_eq!(human.status, 255);
    assert!(human.stderr.contains("USAGE_ERROR"), "{:?}", human.stderr);
    assert!(
        human.stdout.is_empty(),
        "a usage error must not print operands: {:?}",
        human.stdout
    );
    harness.assert_no_remote_tool("usage errors")
}

#[test]
fn a_group_without_a_subcommand_reports_its_own_usage_operation() -> Result<(), String> {
    let harness = Harness::open("group-usage")?;
    for group in ["session", "connection", "fs", "tunnel"] {
        let (outcome, value) = envelope(&harness, &[group, "--json"])?;
        assert_eq!(outcome.status, 255);
        assert_eq!(operation(&value), format!("{group}.usage"));
        want_code(&value, "USAGE_ERROR")?;
        assert_eq!(
            value["data"],
            serde_json::Value::Null,
            "a usage envelope carries no data"
        );
        let human = run(&harness, &[group])?;
        assert_eq!(
            human.status, 255,
            "a group alone is a usage error for humans too"
        );
    }
    harness.assert_no_remote_tool("group usage")
}

/// A group's `--help` is that group's own help: a human who asked how to use
/// `session` must not be handed the whole CLI, and the leaf list is the group's
/// contract with them. With `--json` the same invocation is a usage error under
/// the group's own operation, because prose on stdout would break a decoder
/// (WIRE-002).
#[test]
fn group_help_lists_that_group_and_json_help_is_that_groups_usage_error() -> Result<(), String> {
    let harness = Harness::open("group-help")?;
    for (group, leaf) in [
        ("session", "recover"),
        ("connection", "reset"),
        ("fs", "mirror"),
        ("tunnel", "close"),
    ] {
        let help = run(&harness, &[group, "--help"])?;
        assert_eq!(help.status, 0, "{group}: {}", help.stderr);
        assert!(
            help.stdout
                .starts_with(&format!("Usage: rhost {group} <subcommand>")),
            "{group} help is not its own: {:?}",
            help.stdout
        );
        assert!(
            help.stdout.contains(&format!("{group} {leaf}")),
            "{group} help omits {leaf}: {:?}",
            help.stdout
        );
        assert!(help.stderr.is_empty(), "{group}: {:?}", help.stderr);

        let (outcome, value) = envelope(&harness, &[group, "--json", "--help"])?;
        assert_eq!(outcome.status, 255);
        assert_eq!(operation(&value), format!("{group}.usage"));
        want_code(&value, "USAGE_ERROR")?;
    }
    // The root is still the root's own usage, and it names every group.
    let root = run(&harness, &["--help"])?;
    assert_eq!(root.status, 0);
    for line in [
        "rhost session create|list|exec|send|read|recover|close",
        "rhost audit [--limit N] [--host HOST]",
    ] {
        assert!(root.stdout.contains(line), "{line}: {:?}", root.stdout);
    }
    harness.assert_no_remote_tool("help")
}

#[test]
fn a_whitespace_command_is_configuration_not_usage_and_never_reaches_ssh() -> Result<(), String> {
    let harness = Harness::open("blank-command")?;
    harness.stub("ssh", "exit 99")?;
    for command in ["   ", "\t"] {
        let want = "CONFIG_INVALID";
        let (outcome, value) = envelope(
            &harness,
            &["exec", "gpu", "--json", "--fresh", "--command", command],
        )?;
        assert_eq!(outcome.status, 255);
        assert_eq!(&operation(&value), "exec");
        want_code(&value, want)?;
        assert_eq!(
            value
                .pointer("/data/execution/status")
                .and_then(serde_json::Value::as_str),
            Some("not_started"),
            "rejected input must not claim a remote state"
        );
        assert_eq!(
            value
                .pointer("/data/cleanup/status")
                .and_then(serde_json::Value::as_str),
            Some("not_attempted")
        );
        assert_eq!(value["error"]["retryable"], serde_json::Value::Bool(false));
    }
    harness.assert_no_remote_tool("whitespace command")
}

#[test]
fn out_of_range_values_are_configuration_errors() -> Result<(), String> {
    let harness = Harness::open("ranges")?;
    harness.stub("ssh", "exit 99")?;
    for extra in [
        vec!["--timeout", "-5s"],
        vec!["--max-output-bytes", "-1"],
        vec!["--max-output-bytes", "67108865"],
        vec!["--env", "BAD"],
        vec!["--env", "=value"],
        vec!["--env", "1X=2"],
    ] {
        let mut args = vec!["exec", "gpu", "--json", "--fresh", "--command", "date"];
        args.extend(extra.iter().copied());
        let (outcome, value) = envelope(&harness, &args)?;
        assert_eq!(outcome.status, 255, "{args:?}");
        want_code(&value, "CONFIG_INVALID").map_err(|error| format!("{args:?} {error}"))?;
        assert_eq!(operation(&value), "exec");
    }
    harness.assert_no_remote_tool("out-of-range values")
}

#[test]
fn a_missing_command_file_fails_locally_before_any_submission() -> Result<(), String> {
    let harness = Harness::open("command-file")?;
    harness.stub("ssh", "exit 99")?;
    let missing = harness.path("nowhere.sh");
    let name = missing.to_str().ok_or("fixture path is not UTF-8")?;
    let (outcome, value) = envelope(&harness, &["exec", "gpu", "--json", "--command-file", name])?;
    assert_eq!(outcome.status, 255);
    assert_eq!(operation(&value), "exec");
    want_code(&value, "CONFIG_INVALID")?;
    // A directory is not a program either.
    let dir = harness.path("home");
    let name = dir.to_str().ok_or("fixture path is not UTF-8")?;
    let (_, value) = envelope(&harness, &["exec", "gpu", "--json", "--command-file", name])?;
    want_code(&value, "CONFIG_INVALID")?;
    harness.assert_no_remote_tool("command-file")
}
