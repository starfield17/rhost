//! The human view of a session operation.

use super::ops;
use crate::cli::{Sink, warn};

/// A created session, and the id every later call is resolved against.
pub(crate) fn session_created(sink: &mut Sink, info: &ops::Info) {
    sink.line(&format!("created session {} (name {})", info.id, info.name));
}

/// The sessions this account has records for. An empty list is a statement, not
/// a table with nothing under it.
pub(crate) fn session_list(sink: &mut Sink, rows: &[ops::Info]) {
    if rows.is_empty() {
        warn("no sessions");
        return;
    }
    sink.line("ID  NAME  STATUS  CWD");
    for row in rows {
        sink.line(&format!(
            "{}  {}  {}  {}",
            row.id,
            dash(&row.name),
            row.status.as_str(),
            dash(&row.initial_cwd)
        ));
    }
}

/// Terminal output goes to stdout as-is, so `session read ... > file` stays
/// clean; the envelope carries the same bytes plus the cursor.
pub(crate) fn session_output(sink: &mut Sink, content: &str) {
    sink.text(content);
    if !content.is_empty() && !content.ends_with('\n') {
        sink.line("");
    }
}

/// A missing name or cwd reads as a dash rather than as an empty column.
fn dash(text: &str) -> &str {
    if text.is_empty() { "-" } else { text }
}
