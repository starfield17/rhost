//! `audit`: the local trail of remote operations, plus its command.
//!
//! `trail` owns the file — bounded metadata, fail-open writes — and `command`
//! owns both `rhost audit` and the [`Timer`] every other capability records one
//! operation through, so a write that does not land is reported in exactly one
//! place. The `customer` of the trail is every capability; they depend on
//! `Timer`, never on the file format.

mod command;
mod dto;
pub mod trail;

pub use command::{AUDIT_FLAGS, Audit, Timer, command, record, run};
pub use trail::{Entry, Recorder};
