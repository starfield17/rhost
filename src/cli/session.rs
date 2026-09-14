//! The `session` command surface: grammar, running and the process status each
//! answer implies.
//!
//! The session itself belongs to remote tmux, so nothing here is remembered
//! between calls: every subcommand is one helper submission against records the
//! remote host owns. What this module adds is the agent-facing contract — a
//! canonical id, a status to branch on, an incremental cursor, and the rule that a
//! refused command never reaches the pane.
//!
//! `parse` turns argv into an operation and owns the flag tables; `run` executes
//! one against the host and picks the delivery the caller asked for.

mod parse;
mod run;

pub use parse::Session;
pub(crate) use parse::command;
pub(crate) use run::run;
