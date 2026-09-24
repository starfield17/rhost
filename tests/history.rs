use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;

#[path = "support/test_pointer.rs"]
mod test_pointer;

#[derive(Debug, Deserialize)]
struct HistoricalRow {
    id: String,
    origin: String,
    contract: String,
    mechanism_test: String,
    conformance_test: Option<String>,
    #[serde(default)]
    gap: String,
}

#[test]
fn historical_regressions_name_live_contracts_and_tests() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let rows: Vec<HistoricalRow> = serde_json::from_str(include_str!("fixtures/history.json"))
        .map_err(|error| format!("parse history: {error}"))?;
    let ledger = std::fs::read_to_string(root.join("docs/CONTRACT.md"))
        .map_err(|error| format!("read contract ledger: {error}"))?;
    let mut ids = BTreeSet::new();

    for row in rows {
        if !row.id.starts_with("HIST-") || !ids.insert(row.id.clone()) {
            return Err(format!(
                "missing or duplicate historical identity: {}",
                row.id
            ));
        }
        if !(7..=40).contains(&row.origin.len())
            || !row.origin.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(format!("{} has invalid origin {}", row.id, row.origin));
        }
        if !ledger.contains(&format!("| {} |", row.contract)) {
            return Err(format!(
                "{} references unknown contract {}",
                row.id, row.contract
            ));
        }
        test_pointer::exists(root, &row.mechanism_test)?;
        if let Some(pointer) = &row.conformance_test {
            test_pointer::exists(root, pointer)?;
        } else if row.gap.trim().is_empty() {
            return Err(format!(
                "{} has no independent test or explicit gap",
                row.id
            ));
        }
    }
    Ok(())
}
