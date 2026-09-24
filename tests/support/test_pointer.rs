use regex::Regex;
use std::path::Path;

/// A ledger pointer is evidence only when it names an active Rust test function.
pub(crate) fn exists(root: &Path, pointer: &str) -> Result<(), String> {
    let (path, symbol) = pointer
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid test pointer {pointer}"))?;
    if !(path.starts_with("src/") || path.starts_with("tests/"))
        || !path.ends_with(".rs")
        || Path::new(path).is_absolute()
        || path.split('/').any(|part| part == "..")
        || !symbol
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(format!("not a Rust regression pointer: {pointer}"));
    }
    let source = std::fs::read_to_string(root.join(path))
        .map_err(|error| format!("read {pointer}: {error}"))?;
    let escaped = regex::escape(symbol);
    let pattern = format!(
        r"(?m)^\s*#\[test\]\s*(?:#\[[^\n]+\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn {escaped}\s*\("
    );
    let declaration = Regex::new(&pattern).map_err(|error| error.to_string())?;
    if !declaration.is_match(&source) {
        return Err(format!("test pointer names no test function: {pointer}"));
    }
    Ok(())
}
