//! Acceptance of the planned request vocabulary, independent of CLI wiring.
use serde_json::{Value, json};

fn validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(include_str!("../schemas/request-v4.schema.json"))
        .unwrap_or_else(|error| panic!("schema JSON: {error}"));
    jsonschema::validator_for(&schema).unwrap_or_else(|error| panic!("schema: {error}"))
}

fn start() -> Value {
    json!({
        "schema_version": 4, "operation": "run.start", "host": "gpu",
        "params": {
            "generation": "a".repeat(32),
            "key": format!("2000000000-{}", "b".repeat(64)),
            "expires_unix": 2000000000u64,
            "cwd": "/srv/project", "program": {"argv": ["printf", "%s", "a; b"]},
            "environment": {"inherit": false, "values": {"PATH": "/usr/bin:/bin"}}
        }
    })
}

#[test]
fn structured_requests_keep_program_boundaries_and_explicit_environment() {
    let schema = validator();
    let request = start();
    assert!(schema.is_valid(&request));
    let mut shell = request.clone();
    shell["params"]["program"] =
        json!({"interpreter": "/bin/sh", "script": "printf '%s' 'a; b'", "login": false});
    assert!(schema.is_valid(&shell));
    for field in [
        "generation",
        "key",
        "expires_unix",
        "cwd",
        "program",
        "environment",
    ] {
        let mut invalid = request.clone();
        invalid["params"]
            .as_object_mut()
            .unwrap_or_else(|| panic!("params"))
            .remove(field);
        assert!(!schema.is_valid(&invalid), "missing {field}");
    }
}

#[test]
fn invalid_or_ambiguous_requests_are_refused_by_the_contract() {
    let schema = validator();
    for (field, value) in [
        ("program", json!({"argv": []})),
        ("program", json!({"argv": [""]})),
        ("program", json!({"argv": ["echo", "\u{0000}"]})),
        ("program", json!({"argv": ["echo"], "script": "echo"})),
        (
            "program",
            json!({"interpreter": "sh", "script": "echo", "login": false}),
        ),
        (
            "environment",
            json!({"inherit": false, "values": {"BAD=KEY": "value"}}),
        ),
        ("wait_ms", json!(-1)),
        ("wait_ms", json!(true)),
        ("stdin_base64", json!("not base64!")),
        ("key", json!("reusable-name")),
        ("deadline_ms", json!(0)),
        ("unknown", json!(true)),
    ] {
        let mut invalid = start();
        invalid["params"][field] = value;
        assert!(
            !schema.is_valid(&invalid),
            "accepted invalid {field}: {invalid}"
        );
    }
    let mut unknown = start();
    unknown["operation"] = json!("run.restart");
    assert!(!schema.is_valid(&unknown));
    let mut old = start();
    old["schema_version"] = json!(2);
    assert!(!schema.is_valid(&old));
}

#[test]
fn control_and_observation_requests_do_not_accept_launch_payloads() {
    let schema = validator();
    for operation in [
        "metadata",
        "run.list",
        "run.prune",
        "run.poll",
        "run.signal",
    ] {
        let mut params = json!({});
        if operation != "metadata" {
            params["generation"] = json!("a".repeat(32));
        }
        if matches!(operation, "run.poll" | "run.signal") {
            params["run_id"] = json!(format!("2000000000-{}", "b".repeat(64)));
        }
        if operation == "run.signal" {
            params["signal"] = json!("TERM");
        }
        let mut request =
            json!({"schema_version": 4, "operation": operation, "host": "gpu", "params": params});
        assert!(schema.is_valid(&request), "{request}");
        request["params"]["program"] = json!({"argv": ["echo"]});
        assert!(!schema.is_valid(&request));
    }
}
