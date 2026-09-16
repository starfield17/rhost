//! The `fs` capability surface: the parsed request type and the names callers
//! use. Kept apart from the grammar so a neighbor imports `files::Fs`, not the
//! parser's internals.

/// Every file operation the parser can hand to the runner.
pub enum Fs {
    Put {
        host: String,
        local: String,
        remote: String,
        timeout_nanos: i64,
        resume: bool,
        checksum: bool,
        parents: bool,
    },
    Get {
        host: String,
        remote: String,
        local: String,
        timeout_nanos: i64,
        resume: bool,
        checksum: bool,
    },
    /// `sync` pushes and `mirror` pulls; the direction is part of the operation
    /// name, never a flag a caller could get backwards.
    Sync {
        mirror: bool,
        host: String,
        source: String,
        destination: String,
        timeout_nanos: i64,
        delete: bool,
        dry_run: bool,
        checksum: bool,
        excludes: Vec<String>,
    },
    Batch {
        host: String,
        manifest: String,
        timeout_nanos: i64,
    },
    Read {
        host: String,
        path: String,
        start: u64,
        lines: u64,
        max_bytes: usize,
        timeout_nanos: i64,
    },
    Write {
        host: String,
        path: String,
        /// `-` means stdin, read before anything is sent.
        from: String,
        if_hash: Option<String>,
        parents: bool,
        mode: Option<String>,
        timeout_nanos: i64,
    },
    Patch {
        host: String,
        path: String,
        patch: String,
        if_hash: Option<String>,
        timeout_nanos: i64,
    },
}
