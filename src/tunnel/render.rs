//! The human view of a tunnel operation.

use crate::cli::{Sink, warn};

/// One tunnel, and the one thing a person has to keep: its id.
pub(crate) fn tunnel(sink: &mut Sink, result: &super::Tunnel) {
    sink.line(&format!(
        "{}  {}  {}  {}  {}",
        result.id,
        result.status.as_str(),
        result.kind.as_str(),
        result.listen,
        dash(result.destination.as_deref().unwrap_or(""))
    ));
    warn(&format!("keep this id: rhost tunnel close {}", result.id));
}

/// The tunnels this machine has records for. An empty list is a statement, not a
/// table with nothing under it.
pub(crate) fn tunnels(sink: &mut Sink, rows: &[super::Tunnel]) {
    if rows.is_empty() {
        warn("no tunnels");
        return;
    }
    sink.line("ID  STATUS  KIND  LISTEN  DESTINATION  HOST");
    for row in rows {
        sink.line(&format!(
            "{}  {}  {}  {}  {}  {}",
            row.id,
            row.status.as_str(),
            row.kind.as_str(),
            row.listen,
            dash(row.destination.as_deref().unwrap_or("")),
            row.host
        ));
    }
}

fn dash(text: &str) -> &str {
    if text.is_empty() { "-" } else { text }
}
