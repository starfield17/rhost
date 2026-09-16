//! Moving files, and editing them where they live.
//!
//! `scp` copies one file in either direction and `rsync` syncs a tree; both are
//! tools the user already has, and neither resolves a host (AGENTS.md §5). The
//! editing surface (`read`/`write`/`patch`) runs an embedded helper on the remote
//! host instead, because a compare-and-swap replacement has to happen next to the
//! file it guards, under the same lock, to be worth anything (FS-004).
//!
//! The cut is by capability and by reason to change: `transfer` moves one file,
//! `sync` moves a tree, `edit` changes a file in place, `batch` sequences
//! transfers, and `remote` holds what all of them share — the single path out to
//! a host and the failure taxonomy every answer is phrased in. Callers only need
//! the names re-exported here.

mod batch;
mod edit;
mod remote;
mod sync;
mod transfer;

pub use batch::batch;
pub use edit::{patch, read, write};
pub use sync::{mirror, sync};
pub use transfer::{get, put};

use crate::files::backend::Change;
use crate::remote::Error;
use std::time::Duration;

/// One single-file copy, in either direction.
pub struct Transfer {
    pub source: String,
    pub destination: String,
    pub backend: &'static str,
    pub size: u64,
    pub multiplexed: bool,
    pub duration_ms: u64,
    pub checksum_verified: bool,
    pub resume_enabled: bool,
}

/// One directory sync. A dry run reports the plan and changes nothing.
pub struct Sync {
    pub source: String,
    pub destination: String,
    pub dry_run: bool,
    pub delete: bool,
    pub multiplexed: bool,
    pub changes: Vec<Change>,
    pub files: u64,
    pub directories: u64,
    pub deletes: u64,
    pub notes: Vec<String>,
    pub duration_ms: u64,
}

/// One entry of a batch report: the manifest row, and what happened to it.
pub struct BatchItem {
    pub operation: &'static str,
    pub source: String,
    pub destination: String,
    /// How long this entry took, whether it arrived or not: the audit trail
    /// records one operation per entry, and a failure's duration is part of it.
    pub duration_ms: u64,
    pub data: Option<Transfer>,
    pub error: Option<Error>,
}

/// The whole batch, in manifest order.
pub struct Batch {
    pub items: Vec<BatchItem>,
    pub succeeded: u64,
    pub failed: u64,
}

pub struct PutOptions<'a> {
    pub host: &'a str,
    /// The local side is spelled out rather than named `local`: a field selector
    /// ending in that word reads as an mDNS host suffix to
    /// `scripts/check-portability.sh`, which is deliberately kept strict
    /// (AGENTS.md §1).
    pub local_path: &'a str,
    pub remote: &'a str,
    pub timeout: Option<Duration>,
    pub resume: bool,
    pub checksum: bool,
    pub parents: bool,
}

pub struct GetOptions<'a> {
    pub host: &'a str,
    pub remote: &'a str,
    pub local_path: &'a str,
    pub timeout: Option<Duration>,
    pub resume: bool,
    pub checksum: bool,
}

pub struct SyncOptions<'a> {
    pub host: &'a str,
    /// For [`sync`] this is the local tree; for [`mirror`] it is the remote one.
    pub source: &'a str,
    /// For [`sync`] this is the remote tree; for [`mirror`] it is the local one.
    pub destination: &'a str,
    pub delete: bool,
    pub dry_run: bool,
    pub excludes: &'a [String],
    pub checksum: bool,
    pub timeout: Option<Duration>,
}

pub struct ReadOptions<'a> {
    pub host: &'a str,
    pub path: &'a str,
    pub start: u64,
    pub lines: u64,
    pub max_bytes: usize,
    pub timeout: Option<Duration>,
}

/// A bounded page of a remote text file, with the hash of the whole file.
pub struct Read {
    pub path: String,
    pub sha256: String,
    pub content: String,
    pub start: u64,
    pub lines: u64,
    pub total_lines: u64,
    pub truncated: bool,
}

pub struct WriteOptions<'a> {
    pub host: &'a str,
    pub path: &'a str,
    pub content: Vec<u8>,
    /// Required to replace an existing file; a missing one is `HASH_REQUIRED`.
    pub if_hash: Option<String>,
    pub parents: bool,
    pub mode: Option<String>,
    pub timeout: Option<Duration>,
}

pub struct Write {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// The helper's editing and reading limit, in bytes.
pub const MAX_HELPER_BYTES: usize = 8 * 1024 * 1024;

/// The helper's own budget. It performs one remote action, so its deadline is
/// about a slow host rather than about the size of a file.
const HELPER_TIMEOUT: Duration = Duration::from_secs(60);

/// A capability question ("is rsync there?") is yes/no and must not depend on
/// the login profile being quiet, so it gets its own generous little budget.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
