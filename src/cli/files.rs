//! The `fs` command surface: grammar, running and prose.
//!
//! `fs` is where "copy this" meets "edit that". Everything that touches the
//! network or a local tool goes through `crate::app::files`; this module only
//! turns argv into requests and results into the delivery the caller asked for.
//!
//! The capability is cut by the three things `fs` does: `parse` turns argv into
//! an operation (and owns the flag tables), `run` executes one against the host,
//! and `input` reads a `write` body or a `patch` document locally, before
//! anything is sent.

mod input;
mod parse;
mod run;

pub use parse::Fs;
pub(crate) use parse::command;
pub(crate) use run::run;
