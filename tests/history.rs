use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;

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
        let mut active_rust = test_pointer_exists(root, &row.mechanism_test)?;
        if let Some(pointer) = &row.conformance_test {
            active_rust |= test_pointer_exists(root, pointer)?;
        } else if row.gap.trim().is_empty() {
            return Err(format!(
                "{} has no independent test or explicit gap",
                row.id
            ));
        }
        if row.id.starts_with("HIST-RS-") && !active_rust {
            return Err(format!("{} has no active Rust regression", row.id));
        }
    }
    Ok(())
}

fn test_pointer_exists(root: &Path, pointer: &str) -> Result<bool, String> {
    let (path, symbol) = pointer
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid test pointer {pointer}"))?;
    if Path::new(path).is_absolute() || path.split('/').any(|part| part == "..") {
        return Err(format!("test pointer leaves repository: {pointer}"));
    }
    let source = std::fs::read_to_string(root.join(path))
        .map_err(|error| format!("read {pointer}: {error}"))?;
    let escaped = regex::escape(symbol);
    let pattern = if path.ends_with(".rs") {
        format!(
            r"(?m)^\s*#\[test\]\s*(?:#\[[^\n]+\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn {escaped}\s*\("
        )
    } else if path.ends_with("_test.go") && symbol.starts_with("Test") {
        format!(r"(?m)^func {escaped}\(\w+ \*testing\.T\)")
    } else {
        return Err(format!("not a Go or Rust regression pointer: {pointer}"));
    };
    let declaration = regex::Regex::new(&pattern).map_err(|error| error.to_string())?;
    if !declaration.is_match(&source) {
        return Err(format!("test pointer names no test function: {pointer}"));
    }
    Ok(path.ends_with(".rs") && (path.starts_with("src/") || path.starts_with("tests/")))
}
