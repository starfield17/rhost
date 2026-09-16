//! The human view of a file operation.
//!
//! Every fact printed here also appears in the envelope, and the JSON path is
//! the one agents read: this exists so a person piping `fs read` into a file
//! gets the page and nothing else (AGENTS.md §6).

use super::backend::Action as SyncAction;
use super::ops;
use crate::cli::{Sink, warn};

/// Display-only size; the envelope keeps the integer byte count.
pub(crate) fn human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let units = ["KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", units[unit])
}

pub(crate) fn transfer(sink: &mut Sink, result: &ops::Transfer) {
    sink.line(&format!(
        "transferred {} via {}",
        human_bytes(result.size),
        result.backend
    ));
    sink.line(&format!("  {}", result.source));
    sink.line(&format!("  -> {}", result.destination));
    if result.checksum_verified {
        // Say so out loud: the point of asking for a verified copy is to see the
        // two ends agree, and silence would read as "not verified".
        warn("  verified: identical SHA-256 at both ends");
    }
    if result.resume_enabled {
        warn("  resumable: rsync partial files in .rhost-partial");
    }
}

/// The plan the envelope carries, one action per line. `mirror` reuses both the
/// result and the format, so the verbs are supplied rather than assumed.
pub(crate) fn sync(sink: &mut Sink, result: &ops::Sync, applied: &str, preview: &str) {
    let head = if result.dry_run { preview } else { applied };
    let delete_note = if result.delete {
        " (with --delete enabled)"
    } else {
        ""
    };
    sink.line(&format!(
        "{head} via rsync{delete_note}{}",
        if result.dry_run {
            " (dry run, nothing copied)"
        } else {
            ""
        }
    ));
    sink.line(&format!("  {} -> {}", result.source, result.destination));
    for change in &result.changes {
        let symbol = match change.action {
            SyncAction::Delete => "-",
            SyncAction::Directory => "d",
            SyncAction::Skip => "s",
            _ => "+",
        };
        sink.line(&format!("  {symbol} {}", change.path));
    }
    for note in &result.notes {
        sink.line(&format!("  ! {note}"));
    }
    if result.changes.is_empty() {
        warn("no files changed");
    } else {
        warn(&format!(
            "{} file action(s), {} directory action(s), {} deletion(s)",
            result.files, result.directories, result.deletes
        ));
    }
    if !result.multiplexed {
        warn("note: rsync could not reuse the multiplexed connection and opened its own");
    }
}

/// The human view of a read is the page itself, so `rhost fs read host file >
/// copy.txt` stays clean; everything else goes to stderr.
pub(crate) fn read(sink: &mut Sink, result: &ops::Read) {
    sink.text(&result.content);
    let last = if result.lines == 0 {
        result.start
    } else {
        result.start + result.lines - 1
    };
    warn(&format!(
        "{}: lines {}-{} of {} ({}...)",
        result.path,
        result.start,
        last,
        result.total_lines,
        &result.sha256[..result.sha256.len().min(12)]
    ));
    if result.truncated {
        warn("output was truncated: raise --lines or --max-bytes");
    }
}

pub(crate) fn write(_sink: &mut Sink, result: &ops::Write) {
    warn(&format!(
        "wrote {} ({} bytes, sha256 {}...)",
        result.path,
        result.bytes,
        &result.sha256[..result.sha256.len().min(12)]
    ));
}

pub(crate) fn batch(sink: &mut Sink, result: &ops::Batch) {
    for item in &result.items {
        match (&item.data, &item.error) {
            (Some(data), None) => sink.line(&format!(
                "{}  {} -> {}  ({})",
                item.operation,
                item.source,
                item.destination,
                human_bytes(data.size)
            )),
            (None, Some(error)) => sink.line(&format!(
                "{}  {} -> {}  {}: {}",
                item.operation, item.source, item.destination, error.code, error.message
            )),
            // Neither is impossible: `batch` builds every item with exactly one
            // of the two, and the line is here so a future change cannot print a
            // success that was never reported.
            _ => sink.line(&format!(
                "{}  {} -> {}  no result",
                item.operation, item.source, item.destination
            )),
        }
    }
    warn(&format!(
        "{} transferred, {} failed",
        result.succeeded, result.failed
    ));
}
