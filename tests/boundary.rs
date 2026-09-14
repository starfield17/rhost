use regex::Regex;

#[test]
fn pure_domain_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let comments = Regex::new(r"(?s)/\*.*?\*/|//[^\n]*")?;
    let strings = Regex::new(r#""(?:\\.|[^"\\])*""#)?;
    let forbidden = Regex::new(
        r"\b(?:serde|serde_json|tokio|unsafe|fs|io|process|net|env|thread|include|include_str|include_bytes)\b|\bcrate\s*::|\bsuper\s*::\s*super\b",
    )?;
    let violation = |source: &str| {
        forbidden.is_match(&strings.replace_all(&comments.replace_all(source, ""), ""))
    };
    let mut count = 0;
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src/domain"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            count += 1;
            assert!(
                !violation(&std::fs::read_to_string(&path)?),
                "{}",
                path.display()
            );
        }
    }
    assert!(count > 0);
    for bad in [
        "use crate::output;",
        "use std::{io, fmt};",
        "use std::process::Command;",
        "use serde::Serialize;",
        "use super::super::output;",
    ] {
        assert!(violation(bad), "guard failed to reject {bad}");
    }
    assert!(!violation(
        "// use serde::Serialize;\nuse std::fmt;\nlet value = \"process\";"
    ));
    Ok(())
}
