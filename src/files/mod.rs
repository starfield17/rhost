//! `files`: move, sync and edit remote files.
//!
//! This is one capability. The surface (the parsed `Fs` and the names callers
//! use) is `surface`; the grammar is `command`, the use-cases are `ops`, the
//! local-tool and remote-helper rules are `backend`, and the schema-v2 view is
//! `dto`.

mod backend;
pub mod command;
mod dto;
mod input;
mod ops;
mod render;
mod run;
mod surface;

pub use backend::Stop;
pub use command::command;
pub use dto::{batch, file_failure, file_read, file_write, sync, transfer};
pub use ops::{
    Batch, BatchItem, GetOptions, Read, ReadOptions, Sync, SyncOptions, Transfer, Write,
    WriteOptions, batch as run_batch, get, mirror, patch, put, read, sync as run_sync, write,
};
pub use surface::Fs;

pub(crate) use run::run;
