#[path = "../examples/contract_cases.rs"]
mod contract_cases;

#[test]
fn actual_dtos_match_independent_schema() -> Result<(), Box<dyn std::error::Error>> {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schemas/result-v2.schema.json"))?;
    let validator = jsonschema::validator_for(&schema)?;
    let cases = contract_cases::cases()?;
    assert_eq!(cases.len(), 38);
    for case in cases {
        assert!(validator.is_valid(&case), "{case}");
        let mut missing = case.clone();
        missing
            .as_object_mut()
            .ok_or("not an object")?
            .remove("schema_version");
        assert!(!validator.is_valid(&missing));
    }
    Ok(())
}
