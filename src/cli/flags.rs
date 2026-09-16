//! Flag specifications and the metadata shared with help.

/// One flag the grammar knows.
#[derive(Clone, Copy)]
pub struct FlagSpec {
    pub name: &'static str,
    pub short: Option<char>,
    pub valued: bool,
    /// Reject a second occurrence. Only the two flags that carry a shell program
    /// are unique: silently letting the last `--cwd` win is worse than refusing,
    /// while a repeated boolean is harmless.
    pub unique: bool,
    /// The one line `--help` prints for this flag. It lives beside the flag so
    /// the help renderer can only describe flags the table holds, and a flag
    /// cannot be added without the line a human reads.
    pub help: &'static str,
}

impl FlagSpec {
    pub const fn long(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            short: None,
            valued: false,
            unique: false,
            help,
        }
    }
    pub const fn value(name: &'static str, unique: bool, help: &'static str) -> Self {
        Self {
            name,
            short: None,
            valued: true,
            unique,
            help,
        }
    }
    pub const fn value_short(
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
    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn short(&self) -> Option<char> {
        self.short
    }
    pub fn valued(&self) -> bool {
        self.valued
    }
    pub fn help(&self) -> &'static str {
        self.help
    }
}

pub const JSON: FlagSpec = FlagSpec::long("json", "one machine-readable JSON document on stdout");

pub static ROOT_FLAGS_ALL: &[FlagSpec] = &[
    JSON,
    FlagSpec::long("version", "print the version and exit"),
];

pub static EXEC_FLAGS_ALL: &[FlagSpec] = &[
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

pub static PLAIN_FLAGS: &[FlagSpec] = &[JSON];
pub static DOCTOR_FLAGS_ALL: &[FlagSpec] = &[
    JSON,
    FlagSpec::value("timeout", false, "probe budget; 0 uses the default"),
    FlagSpec::long("fresh", "use an independent SSH connection"),
];

pub fn doctor_flags() -> &'static [FlagSpec] {
    DOCTOR_FLAGS_ALL
}

pub fn exec_flags() -> &'static [FlagSpec] {
    EXEC_FLAGS_ALL
}
