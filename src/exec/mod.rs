//! `exec`: one exact shell program on a remote host.
//!
//! The capability owns everything about `rhost exec` — its grammar, its runner,
//! and the human status line — so a change to how a foreground run is phrased or
//! delivered touches only this directory. What a finished run *means* stays in
//! `crate::remote` (the one submit-a-program path every capability shares), and
//! how it is serialized stays in `crate::wire`.

mod command;
mod run;

pub use command::command;
pub use run::run;

use crate::cli::CommandText;

/// Upper bound on a `--command-file`. A shell program this long is already
/// unusual; the bound exists so a wrong path cannot read a whole volume into
/// memory.
pub(crate) const MAX_COMMAND_FILE_BYTES: u64 = 64 * 1024;

/// The limit `--max-output-bytes` may name, spelled as the CLI states it.
pub(crate) const MAX_OUTPUT_LIMIT: i64 = 64 * 1024 * 1024;

/// Everything `rhost exec` accepted, already validated as grammar.
pub struct Exec {
    pub host: String,
    pub program: CommandText,
    pub cwd: Option<String>,
    /// Raw `KEY=VALUE` operands, split and validated before anything runs.
    pub env: Vec<String>,
    /// `--timeout` in nanoseconds; `0` means no deadline, and a negative value
    /// is a configuration error rather than a parse error (EXEC-002).
    pub timeout_nanos: i64,
    /// `--max-output-bytes` as typed. `None` means the caller did not name a
    /// limit, which differs from naming zero (keep everything).
    pub max_output_bytes: Option<i64>,
    pub fresh: bool,
    pub stream: bool,
}
