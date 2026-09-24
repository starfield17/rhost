//! The semantic ledger must point to active tests, not merely files that exist.

use regex::Regex;
use std::collections::BTreeSet;
use std::path::Path;

#[path = "support/test_pointer.rs"]
mod test_pointer;

#[test]
fn every_contract_row_names_active_evidence() -> Result<(), String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ledger = std::fs::read_to_string(root.join("docs/CONTRACT.md"))
        .map_err(|error| format!("read contract ledger: {error}"))?;
    let row = Regex::new(r"^\|\s*([A-Z]+-[0-9]{3})\s*\|").map_err(|e| e.to_string())?;
    let link = Regex::new(r"\[[^\]]+\]\(([^)]+)\)").map_err(|e| e.to_string())?;
    let verbose = std::env::var_os("CONTRACT_EVIDENCE_VERBOSE").is_some();
    let mut ids = BTreeSet::new();

    for line in ledger.lines() {
        let Some(found) = row.captures(line) else {
            continue;
        };
        let id = &found[1];
        if !ids.insert(id.to_string()) {
            return Err(format!("duplicate contract ID {id}"));
        }
        let evidence = line
            .rsplit('|')
            .nth(1)
            .ok_or_else(|| format!("{id}: missing evidence column"))?;
        let mut active_test = false;
        for found in link.captures_iter(evidence) {
            let target = &found[1];
            if ["https://", "http://", "mailto:"]
                .iter()
                .any(|prefix| target.starts_with(prefix))
            {
                continue;
            }
            let without_anchor = target.split('#').next().unwrap_or("");
            let (local_path, symbol) = match without_anchor.rsplit_once(':') {
                Some((path, symbol)) if path.ends_with(".rs") => (path, Some(symbol)),
                _ => (without_anchor, None),
            };
            let canonical = root
                .join("docs")
                .join(local_path)
                .canonicalize()
                .map_err(|error| format!("{id}: missing {target}: {error}"))?;
            if !canonical.starts_with(root) {
                return Err(format!("{id}: evidence leaves repository: {target}"));
            }
            if let Some(symbol) = symbol {
                let relative = canonical
                    .strip_prefix(root)
                    .map_err(|error| error.to_string())?
                    .to_string_lossy();
                test_pointer::exists(root, &format!("{relative}:{symbol}"))
                    .map_err(|error| format!("{id}: {error}"))?;
                active_test = true;
            }
            if verbose {
                println!("{id}: {target} exists; active_test={}", symbol.is_some());
            }
        }
        let exception = evidence.contains("[no-active-test]");
        if !active_test && !exception {
            return Err(format!(
                "{id}: add an active Rust test pointer or [no-active-test] with a reason"
            ));
        }
        if exception && !evidence.contains("Reason:") {
            return Err(format!("{id}: [no-active-test] needs a Reason:"));
        }
    }
    if ids.is_empty() {
        return Err("contract ledger contains no rows".to_string());
    }
    if verbose {
        println!("contract evidence: {} rows", ids.len());
    }
    Ok(())
}
