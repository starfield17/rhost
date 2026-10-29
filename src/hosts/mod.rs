//! Local OpenSSH alias discovery and its delivery adapters.
mod application;
mod command;
mod dto;
pub(crate) use application::observe;
pub use command::{Hosts, command, run};
pub(crate) use dto::hosts as envelope;
