//! Migration foundation: pure domain states, schema-v2 serialization, and the
//! OpenSSH-backed capabilities that produce them.
//!
//! The tree is cut by capability: each top-level directory owns one reason to
//! change (`exec`, `files`, `session`, ...), and the shared foundation they all
//! rest on is `domain` (pure values), `transport` (the OpenSSH child),
//! `remote` (the one submit-a-program path and the error taxonomy) and `wire`
//! (the schema-v2 envelope). `cli` holds the flag/argv primitives every
//! capability parses with, and `dispatch` is the thin composition root.
#![forbid(unsafe_code)]

mod base64;
mod clock;
mod random;

pub mod audit;
pub mod cli;
pub mod config;
pub mod connection;
pub mod dispatch;
pub mod doctor;
pub mod domain;
pub mod exec;
pub mod files;
pub mod host;
pub mod hosts;
pub mod remote;
pub mod session;
pub mod shell;
pub mod signals;
pub mod stdio;
pub mod transport;
pub mod tunnel;
pub mod wire;
