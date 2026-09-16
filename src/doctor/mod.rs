//! `doctor`: probe a host and report what this execution environment offers.
//!
//! The probe is read-only and runs through the same submit-a-program path as
//! real work, so a successful probe also proves batch authentication, a usable
//! bash and completion evidence all work. `doctor` reports the control-master
//! state using the shared `connection` DTO shape.

mod app;
mod command;
mod dto;
mod run;

pub use app::{CAPABILITIES, Options, Probe, Report, probe};
pub use command::command;
pub use dto::{DoctorDto, doctor, doctor_failure};
pub use run::run;

/// The probe budget a caller who said nothing gets. A probe that never finishes
/// is a failure, not an open-ended wait.
pub(crate) const DEFAULT_TIMEOUT_NANOS: i64 = 60_000_000_000;

/// Everything `rhost doctor` accepted.
pub struct Doctor {
    pub host: String,
    /// `--timeout` in nanoseconds. `0` means the default probe budget, and a
    /// negative value is a configuration error.
    pub timeout_nanos: i64,
    pub fresh: bool,
}
