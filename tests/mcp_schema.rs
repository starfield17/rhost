//! Advertised schemas are specializations of the authoritative contract.
#[path = "../src/mcp/schema.rs"]
mod schema;
use serde_json::{Value, json};
#[test]
fn tool_schemas_preserve_contract_acceptance_and_stay_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let input: Value = serde_json::from_str(include_str!("../schemas/request-v3.schema.json"))?;
    let result: Value = serde_json::from_str(include_str!("../schemas/result-v3.schema.json"))?;
    let fixtures: Value = serde_json::from_str(include_str!("../tests/fixtures/result-v3-valid.json"))?;
    let full = jsonschema::validator_for(&result)?;
    let mut bytes = 0;
    for operation in input["properties"]["operation"]["enum"].as_array().ok_or("ops")? {
        let op = operation.as_str().ok_or("op")?;
        let advertised = schema::output(&result, op)?;
        bytes += serde_json::to_vec(&advertised)?.len();
        bytes += serde_json::to_vec(&schema::input(&input, op)?)?.len();
        let validator = jsonschema::validator_for(&advertised)?;
        for fixture in fixtures.as_array().ok_or("fixtures")? {
            let case = fixture.get("result").unwrap_or(fixture);
            if case["operation"] != op { continue; }
            assert_eq!(validator.is_valid(case), full.is_valid(case), "{op}: {case}");
            for field in ["schema_version", "data", "error", "ok", "operation"] {
                let mut broken = case.clone();
                broken.as_object_mut().ok_or("object")?.remove(field);
                assert_eq!(validator.is_valid(&broken), full.is_valid(&broken), "{op} missing {field}");
            }
            let mut extra = case.clone(); extra["surprise"] = json!(true);
            assert!(!validator.is_valid(&extra));
        }
        assert!(!validator.is_valid(&json!({"schema_version":3,"operation":"unrelated","ok":false,"data":null,"error":null})));
    }
    assert!(bytes < 160 * 1024, "advertised schemas occupy {bytes} bytes");
    Ok(())
}
