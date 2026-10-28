//! Planned Run evidence is distinct from transport and stream completion.
use serde_json::{Value, json};

fn validator() -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(include_str!("../schemas/result-v3.schema.json"))
        .unwrap_or_else(|error| panic!("schema JSON: {error}"));
    jsonschema::validator_for(&schema).unwrap_or_else(|error| panic!("schema: {error}"))
}

fn running() -> Value {
    let stream = json!({"from": 0, "next": 0, "observed_bytes": 0, "content_base64": "", "open": true, "incomplete": false});
    json!({"schema_version": 3, "operation": "run.poll", "ok": true, "host": "gpu", "error": null,
        "data": {"run": {"generation": "a".repeat(32), "run_id": format!("2000000000-{}", "b".repeat(64)), "status": "running"}, "stdout": stream, "stderr": stream, "stale": false}})
}

#[test]
fn completion_does_not_imply_stream_eof_and_signal_is_not_an_exit_code() {
    let schema = validator();
    let mut result = running();
    assert!(schema.is_valid(&result));
    result["data"]["run"]["status"] = json!("exited");
    assert!(!schema.is_valid(&result));
    result["data"]["run"]["completion"] = json!({"exit_code": 0});
    assert!(schema.is_valid(&result));
    result["data"]["run"]["completion"] = json!({"signal": 15});
    assert!(schema.is_valid(&result));
    result["data"]["run"]["completion"] = json!({"signal": 15, "exit_code": 143});
    assert!(!schema.is_valid(&result));
}

#[test]
fn unknown_execution_never_carries_a_guessed_completion() {
    let schema = validator();
    for state in ["accepted", "starting", "running", "lost"] {
        let mut result = running();
        result["data"]["run"]["status"] = json!(state);
        assert!(schema.is_valid(&result));
        result["data"]["run"]["completion"] = json!({"exit_code": 0});
        assert!(!schema.is_valid(&result));
    }
}

#[test]
fn disconnected_observations_are_stale_failures_not_running_success() {
    let schema = validator();
    let result = running();
    let mut failure = result.clone();
    failure["ok"] = json!(false);
    failure["error"] =
        json!({"code": "SSH_UNREACHABLE", "message": "connection unavailable", "retryable": true});
    failure["data"] = json!({"run": result["data"]["run"], "stale": true});
    assert!(schema.is_valid(&failure));
    failure["data"]["stale"] = json!(false);
    assert!(!schema.is_valid(&failure));
    failure["data"] = Value::Null;
    assert!(schema.is_valid(&failure));
    failure["ok"] = json!(true);
    assert!(!schema.is_valid(&failure));
}

#[test]
fn usage_and_expiry_failures_have_closed_machine_readable_envelopes() {
    let schema = validator();
    for (operation, code) in [("request", "USAGE_ERROR"), ("run.start", "REQUEST_EXPIRED")] {
        let mut result = json!({"schema_version": 3, "operation": operation, "ok": false, "host": null, "data": null,
            "error": {"code": code, "message": "refused", "retryable": false}});
        assert!(schema.is_valid(&result));
        result["error"]["surprise"] = json!(true);
        assert!(!schema.is_valid(&result));
    }
}

#[test]
fn each_operation_owns_its_success_and_failure_payload() {
    let schema = validator();
    let observation = running();
    let record = observation["data"]["run"].clone();
    for (operation, data) in [
        (
            "metadata",
            serde_json::from_str::<Value>(include_str!("fixtures/result-v3-valid.json"))
                .unwrap_or_else(|e| panic!("fixtures: {e}"))
                .as_array()
                .and_then(|items| items.iter().find(|f| f["name"] == "metadata"))
                .map(|f| f["result"]["data"].clone())
                .unwrap_or_else(|| panic!("metadata fixture")),
        ),
        ("run.list", json!([record.clone()])),
        ("run.prune", json!({"removed": [record["run_id"]]})),
        (
            "run.signal",
            json!({"run_id": record["run_id"], "delivered": true}),
        ),
    ] {
        let mut result = json!({"schema_version": 3, "operation": operation, "ok": true, "host": "gpu", "data": data, "error": null});
        assert!(schema.is_valid(&result), "{result}");
        result["ok"] = json!(false);
        result["error"] =
            json!({"code": "STATE_UNAVAILABLE", "message": "unavailable", "retryable": false});
        result["data"] = json!({"run": record, "stale": true});
        assert!(
            !schema.is_valid(&result),
            "foreign stale payload: {operation}"
        );
        result["data"] = Value::Null;
        assert!(schema.is_valid(&result));
    }
}
