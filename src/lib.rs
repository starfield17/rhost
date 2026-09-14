//! Migration foundation: pure domain states, schema-v2 serialization, and the
//! OpenSSH-backed capabilities that produce them.
#![forbid(unsafe_code)]

mod base64;
mod clock;
mod random;

pub mod app;
pub mod audit;
pub mod cli;
pub mod config;
pub mod domain;
pub mod fileops;
pub mod host;
pub mod output;
pub mod session;
pub mod shell;
pub mod signals;
pub mod stdio;
pub mod transport;
pub mod tunnel;
