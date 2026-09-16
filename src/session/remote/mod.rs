//! The remote side of a session: the helper programs and their line protocol.
//!
//! These are rhost's own programs, not a foreign dialect: `scripts` holds one
//! complete shell program per operation and `protocol` parses what it prints.
//! Neither is a capability on its own — they are internals of `session`, private
//! so a neighbor names a session operation rather than a helper line.

pub mod protocol;
pub mod scripts;
