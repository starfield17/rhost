//! Editing a file where it lives: read a bounded page, write or patch behind a
//! hash precondition.
//!
//! The work happens in an embedded helper on the remote host, because a
//! compare-and-swap replacement is only worth anything if it happens next to the
//! file it guards, under the same lock (FS-004). The helper's own reading and
//! writing limits are enforced here too, so a hopeless request fails before it is
//! encoded and sent.

use super::super::Error;
use super::super::exec;
use super::remote::{helper, internal, validation};
use super::{HELPER_TIMEOUT, MAX_HELPER_BYTES, Read, ReadOptions, Write, WriteOptions};
use crate::fileops::{self, remote};
use crate::transport::openssh::Client;
use serde_json::Value;
use std::time::Duration;

/// A bounded page of a remote text file, plus the hash of the whole file.
pub fn read(client: &Client, options: &ReadOptions<'_>) -> Result<Read, Error> {
    fileops::validate_remote_path(options.path).map_err(validation)?;
    if options.max_bytes == 0 || options.max_bytes > MAX_HELPER_BYTES {
        return Err(Error::new(
            "CONFIG_INVALID",
            format!("--max-bytes must be between 1 and {MAX_HELPER_BYTES}"),
        ));
    }
    if options.start < 1 || options.lines < 1 {
        return Err(Error::new(
            "CONFIG_INVALID",
            "--start and --lines must be positive",
        ));
    }
    let answer = helper(
        client,
        options.host,
        &remote::read_request(
            options.path,
            options.start,
            options.lines,
            options.max_bytes,
        ),
        options.timeout.unwrap_or(HELPER_TIMEOUT),
        options.max_bytes,
    )?;
    let page = remote::read_result(&answer).map_err(internal)?;
    Ok(Read {
        path: page.path,
        sha256: page.sha256,
        content: page.content,
        start: page.start,
        lines: page.lines,
        total_lines: page.total_lines,
        truncated: page.truncated,
    })
}

/// The permission syntax the helper accepts: three or four octal digits.
pub(crate) fn is_octal_mode(mode: &str) -> bool {
    matches!(mode.len(), 3 | 4) && mode.bytes().all(|byte| (b'0'..=b'7').contains(&byte))
}

/// Creates, or hash-guarded replaces, a remote text file.
pub fn write(client: &Client, options: &WriteOptions<'_>) -> Result<Write, Error> {
    fileops::validate_remote_path(options.path).map_err(validation)?;
    if options.content.len() > MAX_HELPER_BYTES {
        return Err(Error::new(
            "FILE_TOO_LARGE",
            "input exceeds the 8 MiB editing limit",
        ));
    }
    // The helper enforces this too; checking here costs nothing and saves a
    // round trip on a request that cannot succeed.
    if let Some(mode) = &options.mode {
        if !is_octal_mode(mode) {
            return Err(Error::new(
                "CONFIG_INVALID",
                "--mode must be an octal permission such as 0644",
            ));
        }
    }
    let answer = helper(
        client,
        options.host,
        &remote::write_request(
            options.path,
            &options.content,
            options.if_hash.as_deref().unwrap_or(""),
            options.parents,
            options.mode.as_deref(),
        ),
        options.timeout.unwrap_or(HELPER_TIMEOUT),
        exec::DEFAULT_JSON_CAPTURE,
    )?;
    let written = remote::write_result(&answer).map_err(internal)?;
    Ok(Write {
        path: written.path,
        sha256: written.sha256,
        bytes: written.bytes,
    })
}

pub fn patch(
    client: &Client,
    host: &str,
    path: &str,
    edits: Vec<Value>,
    if_hash: Option<String>,
    max_bytes: usize,
    timeout: Option<Duration>,
) -> Result<Write, Error> {
    fileops::validate_remote_path(path).map_err(validation)?;
    if edits.is_empty() {
        return Err(Error::new("CONFIG_INVALID", "patch document has no edits"));
    }
    let expected = match if_hash {
        Some(hash) if !hash.is_empty() => hash,
        _ => {
            return Err(Error::new(
                "CONFIG_INVALID",
                "patch document has no sha256: read the file first and carry its hash",
            ));
        }
    };
    let answer = helper(
        client,
        host,
        &remote::patch_request(path, edits, &expected, max_bytes),
        timeout.unwrap_or(HELPER_TIMEOUT),
        max_bytes,
    )?;
    let written = remote::write_result(&answer).map_err(internal)?;
    Ok(Write {
        path: written.path,
        sha256: written.sha256,
        bytes: written.bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octal_modes_are_the_only_permissions_that_travel() {
        assert!(is_octal_mode("0644"));
        assert!(is_octal_mode("755"));
        assert!(is_octal_mode("0000"));
        assert!(is_octal_mode("7777"));
        assert!(!is_octal_mode("abc"));
        assert!(!is_octal_mode("08"));
        assert!(!is_octal_mode("07555"));
        assert!(!is_octal_mode(""));
    }
}
