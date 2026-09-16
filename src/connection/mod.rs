//! `connection`: observe and stop the shared OpenSSH control master.
//!
//! A shared master is OpenSSH's, not rhost's; this capability only reports what
//! the master says about itself and, on `reset`, asks it to stop accepting new
//! channels. Accepted channels are left alone.

mod command;
mod dto;
mod run;

pub use command::command;
pub use dto::{ConnectionDto, connection, connection_failure};
pub use run::{reset, run, status};

/// Everything `rhost connection` accepted.
pub enum Connection {
    Status { host: String },
    Reset { host: String },
}
