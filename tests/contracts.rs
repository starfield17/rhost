#[path = "../examples/contract_cases.rs"]
mod contract_cases;

#[test]
fn actual_dtos_match_independent_schema() -> Result<(), Box<dyn std::error::Error>> {
    let cases = contract_cases::cases()?;
    assert_eq!(cases.len(), 38);
    for (name, raw) in [
        (
            "active v2 schema",
            include_str!("../schemas/result-v2.schema.json"),
        ),
        (
            "published v4.1.0 v2 validator",
            include_str!("../schemas/compat/result-v2-v4.1.0.schema.json"),
        ),
    ] {
        let schema: serde_json::Value = serde_json::from_str(raw)?;
        let validator = jsonschema::validator_for(&schema)?;
        for case in &cases {
            assert!(validator.is_valid(case), "{name} rejected {case}");
            let mut missing = case.clone();
            missing
                .as_object_mut()
                .ok_or("not an object")?
                .remove("schema_version");
            assert!(!validator.is_valid(&missing));
        }
    }
    Ok(())
}

#[test]
fn published_v2_validator_detects_closed_object_growth() -> Result<(), Box<dyn std::error::Error>> {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../schemas/compat/result-v2-v4.1.0.schema.json"
    ))?;
    let validator = jsonschema::validator_for(&schema)?;
    let mut version = contract_cases::cases()?
        .into_iter()
        .find(|case| case["operation"] == "version" && case["ok"] == true)
        .ok_or("missing successful version case")?;
    version["data"]
        .as_object_mut()
        .ok_or("version data is not an object")?
        .insert("future_required_field".to_string(), true.into());
    assert!(
        !validator.is_valid(&version),
        "a v2 closed-object addition must bump the schema generation"
    );
    Ok(())
}

#[test]
fn published_v410_validator_is_immutable() {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(include_bytes!(
        "../schemas/compat/result-v2-v4.1.0.schema.json"
    ));
    assert_eq!(
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "94da74d3cf1b9857fe64b2c399947eb4274d79465fe9fff197f3af55a00b50f3"
    );
}
