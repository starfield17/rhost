//! `audit`: the local trail of remote operations, plus its command.
//!
//! `trail` owns the file — bounded metadata, fail-open writes — and `command`
//! owns `rhost audit`. `record` owns the shared [`Timer`] and fail-open recording
//! used by capabilities independently of CLI presentation. The `customer` of the trail is every capability; they depend on
//! `Timer`, never on the file format.

mod command;
mod dto;
mod record;
pub mod trail;

pub use command::{AUDIT_FLAGS, Audit, command, run};
pub use trail::{Entry, Recorder};

pub use record::{Timer, record};
