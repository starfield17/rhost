//! The file-operation envelopes: one copy, a sync plan, a batch report, and the
//! bounded read/write results.

use super::ops;
use crate::wire::{Envelope, ErrorPayload, error_payload, failure};
use serde::Serialize;

/// One single-file copy, with the endpoints that actually hold the bytes.
#[derive(Debug, Serialize)]
pub struct TransferDto<'a> {
    source: &'a str,
    destination: &'a str,
    backend: &'a str,
    size: u64,
    multiplexed: bool,
    duration_ms: u64,
    checksum_verified: bool,
    resume_enabled: bool,
}

impl<'a> From<&'a ops::Transfer> for TransferDto<'a> {
    fn from(value: &'a ops::Transfer) -> Self {
        Self {
            source: &value.source,
            destination: &value.destination,
            backend: value.backend,
            size: value.size,
            multiplexed: value.multiplexed,
            duration_ms: value.duration_ms,
            checksum_verified: value.checksum_verified,
            resume_enabled: value.resume_enabled,
        }
    }
}

pub fn transfer<'a>(
    operation: &'static str,
    host: &str,
    result: &'a ops::Transfer,
) -> Envelope<TransferDto<'a>> {
    Envelope {
        schema_version: 2,
        operation,
        ok: true,
        host: Some(host.to_string()),
        error: None,
        data: result.into(),
    }
}

/// One line of rsync's itemized plan.
#[derive(Debug, Serialize)]
pub struct ChangeDto<'a> {
    action: &'static str,
    path: &'a str,
    itemize: &'a str,
}

#[derive(Debug, Serialize)]
pub struct SyncDto<'a> {
    source: &'a str,
    destination: &'a str,
    backend: &'static str,
    dry_run: bool,
    delete: bool,
    multiplexed: bool,
    changes: Vec<ChangeDto<'a>>,
    files: u64,
    directories: u64,
    deletes: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    notes: Vec<&'a str>,
    duration_ms: u64,
}

pub fn sync<'a>(
    operation: &'static str,
    host: &str,
    result: &'a ops::Sync,
) -> Envelope<SyncDto<'a>> {
    Envelope {
        schema_version: 2,
        operation,
        ok: true,
        host: Some(host.to_string()),
        error: None,
        data: SyncDto {
            source: &result.source,
            destination: &result.destination,
            backend: "rsync",
            dry_run: result.dry_run,
            delete: result.delete,
            multiplexed: result.multiplexed,
            changes: result
                .changes
                .iter()
                .map(|change| ChangeDto {
                    action: change.action.as_str(),
                    path: &change.path,
                    itemize: &change.itemize,
                })
                .collect(),
            files: result.files,
            directories: result.directories,
            deletes: result.deletes,
            notes: result.notes.iter().map(String::as_str).collect(),
            duration_ms: result.duration_ms,
        },
    }
}

/// One entry of a batch report. `data` and `error` are mutually exclusive by
/// construction, which is what the schema checks.
#[derive(Debug, Serialize)]
pub struct BatchItemDto<'a> {
    op: &'a str,
    source: &'a str,
    destination: &'a str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<TransferDto<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ErrorPayload>,
}

#[derive(Debug, Serialize)]
pub struct BatchDto<'a> {
    items: Vec<BatchItemDto<'a>>,
    succeeded: u64,
    failed: u64,
}

/// The batch envelope is `ok` when the batch ran to the end; `failed` counts the
/// entries that did not copy. "The batch completed" and "every file arrived" are
/// different claims, and an agent must be able to tell them apart.
pub fn batch<'a>(host: &str, result: &'a ops::Batch) -> Envelope<BatchDto<'a>> {
    Envelope {
        schema_version: 2,
        operation: "fs.batch",
        ok: true,
        host: Some(host.to_string()),
        error: None,
        data: BatchDto {
            items: result
                .items
                .iter()
                .map(|item| BatchItemDto {
                    op: item.operation,
                    source: &item.source,
                    destination: &item.destination,
                    ok: item.error.is_none(),
                    data: item.data.as_ref().map(TransferDto::from),
                    error: item
                        .error
                        .as_ref()
                        .map(|error| error_payload(error.code, &error.message, error.retryable)),
                })
                .collect(),
            succeeded: result.succeeded,
            failed: result.failed,
        },
    }
}

#[derive(Debug, Serialize)]
pub struct FileReadDto<'a> {
    path: &'a str,
    sha256: &'a str,
    content: &'a str,
    start: u64,
    lines: u64,
    total_lines: u64,
    truncated: bool,
}

pub fn file_read<'a>(host: &str, result: &'a ops::Read) -> Envelope<FileReadDto<'a>> {
    Envelope {
        schema_version: 2,
        operation: "fs.read",
        ok: true,
        host: Some(host.to_string()),
        error: None,
        data: FileReadDto {
            path: &result.path,
            sha256: &result.sha256,
            content: &result.content,
            start: result.start,
            lines: result.lines,
            total_lines: result.total_lines,
            truncated: result.truncated,
        },
    }
}

#[derive(Debug, Serialize)]
pub struct FileWriteDto<'a> {
    path: &'a str,
    sha256: &'a str,
    bytes: u64,
}

pub fn file_write<'a>(
    operation: &'static str,
    host: &str,
    result: &'a ops::Write,
) -> Envelope<FileWriteDto<'a>> {
    Envelope {
        schema_version: 2,
        operation,
        ok: true,
        host: Some(host.to_string()),
        error: None,
        data: FileWriteDto {
            path: &result.path,
            sha256: &result.sha256,
            bytes: result.bytes,
        },
    }
}

/// A failure envelope whose operation and host come from the use-case error.
pub fn file_failure(
    operation: &'static str,
    host: &str,
    error: &crate::remote::Error,
) -> Envelope<Option<()>> {
    failure(operation, host, error.code, &error.message, error.retryable)
}
