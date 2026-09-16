//! The `audit` envelope: where the trail is, and the entries this call selected.

use crate::wire::Envelope;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AuditDto<'a> {
    path: String,
    entries: &'a [super::trail::Entry],
}

pub fn audit<'a>(
    path: &std::path::Path,
    entries: &'a [super::trail::Entry],
) -> Envelope<AuditDto<'a>> {
    Envelope {
        schema_version: 2,
        operation: "audit",
        ok: true,
        host: None,
        data: AuditDto {
            path: path.display().to_string(),
            entries,
        },
        error: None,
    }
}
