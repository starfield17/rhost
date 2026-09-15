//! Flag specifications and the metadata shared with help.

use super::Scope;

/// One flag the grammar knows.
#[derive(Clone, Copy)]
pub(crate) struct FlagSpec {
    pub(super) name: &'static str,
    pub(super) short: Option<char>,
    pub(super) valued: bool,
    /// Reject a second occurrence. Only the two flags that carry a shell program
    /// are unique: silently letting the last `--cwd` win is worse than refusing,
    /// while a repeated boolean is harmless.
    pub(super) unique: bool,
    /// The one line `--help` prints for this flag. It lives beside the flag so
    /// the help renderer can only describe flags the table holds, and a flag
    /// cannot be added without the line a human reads.
    pub(super) help: &'static str,
}

impl FlagSpec {
    pub(crate) const fn long(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            short: None,
            valued: false,
            unique: false,
            help,
        }
    }
    pub(crate) const fn value(name: &'static str, unique: bool, help: &'static str) -> Self {
        Self {
            name,
            short: None,
            valued: true,
            unique,
            help,
        }
    }
    pub(crate) const fn value_short(
        name: &'static str,
        short: char,
        unique: bool,
        help: &'static str,
    ) -> Self {
        Self {
            name,
            short: Some(short),
            valued: true,
            unique,
            help,
        }
    }
    /// What the help renderer reads: the spelling callers type, whether it takes
    /// a value, and the line that describes it.
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }
    pub(crate) fn short(&self) -> Option<char> {
        self.short
    }
    pub(crate) fn valued(&self) -> bool {
        self.valued
    }
    pub(crate) fn help(&self) -> &'static str {
        self.help
    }
}

pub(crate) const JSON: FlagSpec =
    FlagSpec::long("json", "one machine-readable JSON document on stdout");

pub(crate) static ROOT_FLAGS_ALL: &[FlagSpec] = &[
    JSON,
    FlagSpec::long("version", "print the version and exit"),
];

pub(crate) static EXEC_FLAGS_ALL: &[FlagSpec] = &[
    JSON,
    FlagSpec::value_short("command", 'c', true, "the shell program to run"),
    FlagSpec::value("command-file", true, "read the program from a local file"),
    FlagSpec::value("cwd", false, "working directory on the remote host"),
    FlagSpec::value("timeout", false, "execution deadline; 0 waits forever"),
    FlagSpec::value(
        "max-output-bytes",
        false,
        "captured bytes per stream; 0 = all",
    ),
    FlagSpec::value("env", false, "environment variable KEY=VALUE (repeatable)"),
    FlagSpec::long("fresh", "use an independent SSH connection"),
    FlagSpec::long("stream", "mirror live output to stderr; JSON stays"),
];

pub(crate) static PLAIN_FLAGS: &[FlagSpec] = &[JSON];
pub(crate) static DOCTOR_FLAGS_ALL: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false, "probe budget; 0 uses the default"),
    FlagSpec::long("fresh", "use an independent SSH connection"),
];

/// The flags every scope accepts, plus its own. `--json` is a root persistent
/// flag: it may be written on any command, in any position before `--`.
pub(crate) fn flags(scope: Scope) -> &'static [FlagSpec] {
    // Each table repeats `--json` rather than concatenating at run time, so the
    // flag set stays a compile-time fact.
    match scope {
        Scope::Root => ROOT_FLAGS_ALL,
        Scope::Session | Scope::Connection | Scope::Tunnel | Scope::Fs => PLAIN_FLAGS,
    }
}

pub(crate) fn exec_flags() -> &'static [FlagSpec] {
    EXEC_FLAGS_ALL
}

pub(crate) fn doctor_flags() -> &'static [FlagSpec] {
    DOCTOR_FLAGS_ALL
}
