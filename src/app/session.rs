//! Sessions: a persistent remote shell rhost can come back to.
//!
//! This layer decides what the caller asked for and what the answer means. The
//! session itself belongs to remote tmux, so every use-case here is one helper
//! submission; nothing is remembered between calls, and a name is resolved on the
//! remote side against the records that are actually there.
//!
//! The use-cases are cut by what they do to a session: `create` makes one,
//! `exec` runs one command in it, `io` injects input and reads the log back,
//! `lifecycle` lists, recovers and closes, and `errors` turns a helper's refusal
//! into the error taxonomy. `shared` holds the two capture budgets and the one
//! clock every use-case agrees on.

mod create;
mod errors;
mod exec;
mod io;
mod lifecycle;
mod shared;

pub use create::{CreateOptions, Created, CreationStatus, create};
pub use exec::{ExecOptions, exec};
pub use io::{Read, SendOptions, read, send};
pub use lifecycle::{Info, Recover, close, list, recover};
