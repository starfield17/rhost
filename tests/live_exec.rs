//! Native, schema-v2 live acceptance for the Rust candidate.
//!
//! Cargo only exposes this target with the `live-tests` feature. This keeps the
//! normal local suite hermetic without disabling individual cases. The Make
//! entrypoints require both `RHOST_TEST_HOST` and `RHOST_BIN`; every invocation
//! therefore reaches the caller's explicit host with the caller's exact binary.
#![cfg(unix)]

#[path = "live/audit.rs"]
mod audit;
#[path = "live/exec.rs"]
mod exec;
#[path = "live/files.rs"]
mod files;
#[path = "live/session.rs"]
mod session;
#[path = "live/smoke.rs"]
mod smoke;
#[path = "live/support.rs"]
mod support;
#[path = "live/transport.rs"]
mod transport;
#[path = "live/tunnel.rs"]
mod tunnel;
