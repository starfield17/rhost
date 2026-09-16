//! An ordered list of put and get operations.
//!
//! Convenience orchestration, not a transaction: entries run serially and a
//! failed entry does not stop the ones after it, so the report — not the process
//! status alone — says what arrived.

use super::super::backend::{Stop, Tools};
use super::transfer::{get, put};
use super::{Batch, BatchItem, GetOptions, PutOptions};
use crate::remote::Error;
use crate::transport::Client;
use serde_json::Value;
use std::time::{Duration, Instant};

/// A manifest entry: what to copy, and whether it takes the verified route.
pub(crate) struct ManifestEntry {
    operation: &'static str,
    source: String,
    destination: String,
    resume: bool,
    checksum: bool,
}

pub(crate) fn read_manifest(path: &str) -> Result<Vec<ManifestEntry>, Error> {
    let body = std::fs::read_to_string(path)
        .map_err(|error| Error::new("CONFIG_INVALID", format!("read --manifest: {error}")))?;
    let parsed: Value = serde_json::from_str(&body).map_err(|error| {
        Error::new(
            "CONFIG_INVALID",
            format!("manifest is not a JSON array of {{op,source,destination}}: {error}"),
        )
    })?;
    let rows = parsed.as_array().ok_or_else(|| {
        Error::new(
            "CONFIG_INVALID",
            "manifest is not a JSON array of {op,source,destination}",
        )
    })?;
    if rows.is_empty() {
        return Err(Error::new("CONFIG_INVALID", "manifest lists nothing to do"));
    }
    let mut entries = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let operation = match row.get("op").and_then(Value::as_str) {
            Some("put") => "put",
            Some("get") => "get",
            _ => {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    format!("manifest entry {}: op must be put or get", index + 1),
                ));
            }
        };
        let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or("");
        let (source, destination) = (text("source"), text("destination"));
        if source.is_empty() || destination.is_empty() {
            return Err(Error::new(
                "CONFIG_INVALID",
                format!(
                    "manifest entry {}: source and destination are required",
                    index + 1
                ),
            ));
        }
        entries.push(ManifestEntry {
            operation,
            source: source.to_string(),
            destination: destination.to_string(),
            resume: row.get("resume").and_then(Value::as_bool).unwrap_or(false),
            checksum: row
                .get("checksum")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    Ok(entries)
}

/// Runs an ordered list of put and get operations. It is convenience
/// orchestration, not a transaction: entries run serially and a failed entry
/// does not stop the ones after it.
pub fn batch(
    client: &Client,
    tools: &Tools,
    host: &str,
    manifest: &str,
    timeout: Option<Duration>,
    stop: Stop<'_>,
) -> Result<Batch, Error> {
    let entries = read_manifest(manifest)?;
    let mut report = Batch {
        items: Vec::with_capacity(entries.len()),
        succeeded: 0,
        failed: 0,
    };
    for entry in entries {
        let started = Instant::now();
        let result = match entry.operation {
            "get" => get(
                client,
                tools,
                &GetOptions {
                    host,
                    remote: &entry.source,
                    local_path: &entry.destination,
                    timeout,
                    resume: entry.resume,
                    checksum: entry.checksum,
                },
                stop,
            ),
            _ => put(
                client,
                tools,
                &PutOptions {
                    host,
                    local_path: &entry.source,
                    remote: &entry.destination,
                    timeout,
                    resume: entry.resume,
                    checksum: entry.checksum,
                    parents: false,
                },
                stop,
            ),
        };
        let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let item = match result {
            Ok(data) => {
                report.succeeded += 1;
                BatchItem {
                    operation: entry.operation,
                    source: entry.source,
                    destination: entry.destination,
                    duration_ms,
                    data: Some(data),
                    error: None,
                }
            }
            Err(error) => {
                report.failed += 1;
                BatchItem {
                    operation: entry.operation,
                    source: entry.source,
                    destination: entry.destination,
                    duration_ms,
                    data: None,
                    error: Some(error),
                }
            }
        };
        report.items.push(item);
    }
    Ok(report)
}
