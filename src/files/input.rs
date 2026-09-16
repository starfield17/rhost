//! The local body of a `write` or `patch`, read and bounded before anything is
//! sent remotely.

use crate::remote::Error;
use std::io::{self, Read as _};

/// The editing limit, which is also the helper's own.
const MAX_LOCAL_INPUT_BYTES: u64 = 8 * 1024 * 1024;

/// Reads the body of a `write`, or the document of a `patch`, before anything
/// remote is attempted. `-` means stdin, and the bound is applied here so a
/// hopeless request fails without being encoded and sent.
pub(super) fn read_local_input(name: &str) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    if name == "-" {
        let stdin = io::stdin();
        let mut locked = stdin.lock();
        locked
            .by_ref()
            .take(MAX_LOCAL_INPUT_BYTES + 1)
            .read_to_end(&mut body)
            .map_err(|error| Error::new("CONFIG_INVALID", format!("read stdin: {error}")))?;
    } else {
        body = std::fs::read(name)
            .map_err(|error| Error::new("CONFIG_INVALID", format!("read {name}: {error}")))?;
    }
    if body.len() as u64 > MAX_LOCAL_INPUT_BYTES {
        return Err(Error::new(
            "FILE_TOO_LARGE",
            "input exceeds the 8 MiB editing limit",
        ));
    }
    Ok(body)
}

/// A patch document carries its own hash so it can be reviewed and applied
/// without a human copying a 64-digit token onto a command line. An explicit
/// `--if-hash` overrides it, which is what lets a reviewed file be applied
/// against a freshly read hash.
pub(super) fn parse_patch_document(
    document: &[u8],
    override_hash: Option<&str>,
) -> Result<(Vec<serde_json::Value>, Option<String>), Error> {
    let parsed: serde_json::Value = serde_json::from_slice(document).map_err(|error| {
        Error::new(
            "CONFIG_INVALID",
            format!("patch document is not valid JSON: {error}"),
        )
    })?;
    let edits = parsed
        .get("edits")
        .and_then(serde_json::Value::as_array)
        .filter(|edits| !edits.is_empty())
        .ok_or_else(|| Error::new("CONFIG_INVALID", "patch document has no edits"))?
        .clone();
    let carried = match override_hash {
        Some(hash) if !hash.is_empty() => hash.to_string(),
        _ => parsed
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .filter(|hash| !hash.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                Error::new(
                    "CONFIG_INVALID",
                    "patch document has no sha256: read the file first and carry its hash",
                )
            })?,
    };
    Ok((edits, Some(carried)))
}
