//! Advertised schemas are specializations of the authoritative contract.
#[path = "../src/mcp/schema.rs"]
mod schema;
use serde_json::{Value, json};
#[test]
fn tool_schemas_preserve_contract_acceptance_and_stay_bounded()
-> Result<(), Box<dyn std::error::Error>> {
    let input: Value = serde_json::from_str(include_str!("../schemas/request-v4.schema.json"))?;
    let result: Value = serde_json::from_str(include_str!("../schemas/result-v4.schema.json"))?;
    let fixtures: Value =
        serde_json::from_str(include_str!("../tests/fixtures/result-v4-valid.json"))?;
    let full = jsonschema::validator_for(&result)?;
    let mut bytes = 0;
    for operation in input["properties"]["operation"]["enum"]
        .as_array()
        .ok_or("ops")?
    {
        let op = operation.as_str().ok_or("op")?;
        let advertised = schema::output(&result, op)?;
        bytes += serde_json::to_vec(&advertised)?.len();
        bytes += serde_json::to_vec(&schema::input(&input, op)?)?.len();
        let validator = jsonschema::validator_for(&advertised)?;
        for fixture in fixtures.as_array().ok_or("fixtures")? {
            let case = fixture.get("result").unwrap_or(fixture);
            if case["operation"] != op {
                continue;
            }
            assert_eq!(
                validator.is_valid(case),
                full.is_valid(case),
                "{op}: {case}"
            );
            for field in ["schema_version", "data", "error", "ok", "operation"] {
                let mut broken = case.clone();
                broken.as_object_mut().ok_or("object")?.remove(field);
                assert_eq!(
                    validator.is_valid(&broken),
                    full.is_valid(&broken),
                    "{op} missing {field}"
                );
            }
            let mut extra = case.clone();
            extra["surprise"] = json!(true);
            assert!(!validator.is_valid(&extra));
        }
        assert!(!validator.is_valid(
            &json!({"schema_version":4,"operation":"unrelated","ok":false,"data":null,"error":null})
        ));
    }
    assert!(
        bytes < 160 * 1024,
        "advertised schemas occupy {bytes} bytes"
    );
    Ok(())
}

#[test]
fn tool_input_projection_preserves_target_and_parameter_refusals()
-> Result<(), Box<dyn std::error::Error>> {
    let source: Value = serde_json::from_str(include_str!("../schemas/request-v4.schema.json"))?;
    let full = jsonschema::validator_for(&source)?;
    for (op, params) in [
        ("version", json!({})),
        ("hosts", json!({})),
        ("doctor", json!({"fresh":true})),
        ("connection.status", json!({})),
        ("session.create", json!({})),
        ("session.list", json!({})),
        (
            "session.exec",
            json!({"session":"debug","command":"printf finished"}),
        ),
        (
            "session.send",
            json!({"session":"debug","data":"hello","enter":true}),
        ),
        (
            "session.read",
            json!({"session":"debug","since":0,"max_bytes":4}),
        ),
        ("session.recover", json!({"session":"debug"})),
        ("session.close", json!({"session":"debug"})),
    ] {
        let projected = schema::input(&source, op)?;
        let validator = jsonschema::validator_for(&projected)?;
        let mut valid = json!({"params":params});
        if !matches!(op, "version" | "hosts") {
            valid["host"] = "gpu".into();
        }
        assert!(validator.is_valid(&valid), "{op}: {valid}");
        let mut cases = vec![valid.clone()];
        for (field, value) in [
            ("host", Value::Null),
            ("host", json!("gpu")),
            ("if_boot_id", Value::Null),
            ("if_boot_id", json!("00000000-0000-0000-0000-000000000000")),
            ("extra", json!(true)),
        ] {
            let mut changed = valid.clone();
            changed[field] = value;
            cases.push(changed);
        }
        let mut missing = valid.clone();
        missing.as_object_mut().ok_or("args")?.remove("host");
        cases.push(missing);
        let mut unknown = valid.clone();
        unknown["params"]["extra"] = json!(true);
        cases.push(unknown);
        for args in cases {
            let mut document = args.clone();
            document["schema_version"] = 4.into();
            document["operation"] = op.into();
            assert_eq!(
                validator.is_valid(&args),
                full.is_valid(&document),
                "{op}: {args}"
            );
        }
        let mut guarded = valid.clone();
        guarded["if_boot_id"] = json!("00000000-0000-0000-0000-000000000000");
        assert!(!validator.is_valid(&guarded));
        if matches!(op, "version" | "hosts") {
            let mut targeted = valid.clone();
            targeted["host"] = "gpu".into();
            assert!(!validator.is_valid(&targeted));
        }
    }
    Ok(())
}
