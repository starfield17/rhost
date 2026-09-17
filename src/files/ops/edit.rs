//! Editing a file where it lives: read a bounded page, write or patch behind a
//! hash precondition.
//!
//! The work happens in an embedded helper on the remote host, because a
//! compare-and-swap replacement is only worth anything if it happens next to the
//! file it guards, under the same lock (FS-004). The helper's own reading and
//! writing limits are enforced here too, so a hopeless request fails before it is
//! encoded and sent.

use super::super::backend::{DEFAULT_HELPER_BYTES, Edit, ReadResult, Request, WriteResult};
use super::remote::{helper, validation};
use super::{HELPER_TIMEOUT, MAX_HELPER_BYTES, Read, ReadOptions, Write, WriteOptions};
use crate::remote::Error;
use crate::remote::exec;
use crate::transport::Client;
use std::time::Duration;

/// A bounded page of a remote text file, plus the hash of the whole file.
pub fn read(client: &Client, options: &ReadOptions<'_>) -> Result<Read, Error> {
    super::super::backend::validate_remote_path(options.path).map_err(validation)?;
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
    let page: ReadResult = helper(
        client,
        options.host,
        &Request::Read {
            path: options.path,
            start: options.start,
            lines: options.lines,
            max_bytes: options.max_bytes,
        },
        options.timeout.unwrap_or(HELPER_TIMEOUT),
        options.max_bytes,
    )?;
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
    super::super::backend::validate_remote_path(options.path).map_err(validation)?;
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
    let answer: WriteResult = helper(
        client,
        options.host,
        &Request::Write {
            path: options.path,
            content: crate::base64::encode(&options.content),
            if_hash: options.if_hash.as_deref().unwrap_or(""),
            parents: options.parents,
            file_mode: options.mode.as_deref(),
            max_bytes: DEFAULT_HELPER_BYTES,
        },
        options.timeout.unwrap_or(HELPER_TIMEOUT),
        exec::DEFAULT_JSON_CAPTURE,
    )?;
    Ok(Write {
        path: answer.path,
        sha256: answer.sha256,
        bytes: answer.bytes,
    })
}

pub fn patch(
    client: &Client,
    host: &str,
    path: &str,
    edits: Vec<Edit>,
    if_hash: Option<String>,
    max_bytes: usize,
    timeout: Option<Duration>,
) -> Result<Write, Error> {
    super::super::backend::validate_remote_path(path).map_err(validation)?;
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
    let answer: WriteResult = helper(
        client,
        host,
        &Request::Patch {
            path,
            edits,
            if_hash: &expected,
            max_bytes,
        },
        timeout.unwrap_or(HELPER_TIMEOUT),
        max_bytes,
    )?;
    Ok(Write {
        path: answer.path,
        sha256: answer.sha256,
        bytes: answer.bytes,
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
