//! The local body of a `write` or `patch`, read and bounded before anything is
//! sent remotely.

use super::backend::Edit;
use crate::remote::Error;
use serde_json::Value;
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
///
/// The document's *syntax* is checked here. Its line ranges cannot be checked
/// against an unknown old file until the remote helper reads the authoritative
/// contents, so the helper still owns out-of-bounds decisions.
pub(super) fn parse_patch_document(
    document: &[u8],
    override_hash: Option<&str>,
) -> Result<(Vec<Edit>, Option<String>), Error> {
    let parsed: Value = serde_json::from_slice(document).map_err(|error| {
        Error::new(
            "CONFIG_INVALID",
            format!("patch document is not valid JSON: {error}"),
        )
    })?;
    let raw_edits = parsed
        .get("edits")
        .and_then(Value::as_array)
        .filter(|edits| !edits.is_empty())
        .ok_or_else(|| Error::new("CONFIG_INVALID", "patch document has no edits"))?;
    let carried = match override_hash {
        Some(hash) if !hash.is_empty() => hash.to_string(),
        _ => parsed
            .get("sha256")
            .and_then(Value::as_str)
            .filter(|hash| !hash.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                Error::new(
                    "CONFIG_INVALID",
                    "patch document has no sha256: read the file first and carry its hash",
                )
            })?,
    };

    let mut edits = Vec::with_capacity(raw_edits.len());
    let mut ranges = Vec::with_capacity(raw_edits.len());
    for raw_edit in raw_edits {
        let edit: Edit = serde_json::from_value(raw_edit.clone())
            .map_err(|_| Error::new("INVALID_PATCH", "each edit needs only start, end and text"))?;
        if edit.start < 1 || edit.end < edit.start {
            return Err(Error::new(
                "INVALID_PATCH",
                "edits must be non-overlapping 1-based inclusive ranges inside the file",
            ));
        }
        ranges.push((edit.start, edit.end));
        edits.push(edit);
    }
    ranges.sort_unstable();
    let mut previous_end = 0;
    for (start, end) in ranges {
        if start <= previous_end {
            return Err(Error::new(
                "INVALID_PATCH",
                "edits must be non-overlapping 1-based inclusive ranges inside the file",
            ));
        }
        previous_end = end;
    }
    Ok((edits, Some(carried)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_document_carries_its_hash_and_edits_in_order() -> Result<(), String> {
        let document = br#"{"extra":true,"sha256":"a","edits":[
            {"start":3,"end":3,"text":"late\n"},
            {"start":1,"end":2,"text":"early\n"}
        ]}"#;
        let (edits, hash) = parse_patch_document(document, None).map_err(|error| error.code)?;
        assert_eq!(hash.as_deref(), Some("a"));
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].start, 3);
        assert_eq!(edits[1].start, 1);
        Ok(())
    }

    #[test]
    fn an_explicit_hash_overrides_the_document() -> Result<(), String> {
        let document = br#"{"sha256":"old","edits":[{"start":1,"end":1,"text":"x"}]}"#;
        let (_, hash) =
            parse_patch_document(document, Some("fresh")).map_err(|error| error.code)?;
        assert_eq!(hash.as_deref(), Some("fresh"));
        Ok(())
    }

    #[test]
    fn document_structure_keeps_the_existing_config_answer() {
        for (document, wanted) in [
            (br#"{"edits":[]}"#.as_slice(), "CONFIG_INVALID"),
            (br#"{"edits":{}}"#.as_slice(), "CONFIG_INVALID"),
            (
                br#"{"edits":[{"start":1,"end":1,"text":"x"}]}"#.as_slice(),
                "CONFIG_INVALID",
            ),
        ] {
            let error = parse_patch_document(document, None).err();
            assert_eq!(error.map(|error| error.code), Some(wanted));
        }
    }

    #[test]
    fn malformed_edit_ranges_are_patch_answers_before_any_remote_call() {
        for document in [
            br#"{"sha256":"a","edits":[{"start":1,"end":1}]}"#.as_slice(),
            br#"{"sha256":"a","edits":[{"start":1,"end":1,"text":true}]}"#.as_slice(),
            br#"{"sha256":"a","edits":[{"start":true,"end":1,"text":"x"}]}"#.as_slice(),
            br#"{"sha256":"a","edits":[{"start":2,"end":1,"text":"x"}]}"#.as_slice(),
            br#"{"sha256":"a","edits":[{"start":1,"end":2,"text":"a"},{"start":2,"end":3,"text":"b"}]}"#
                .as_slice(),
        ] {
            let error = parse_patch_document(document, None).err();
            assert_eq!(error.map(|error| error.code), Some("INVALID_PATCH"));
        }
    }
}
