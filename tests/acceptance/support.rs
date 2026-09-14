//! The shared harness: an isolated invocation environment, a recorded stub for
//! every external tool, and the few assertions every case makes.
//!
//! Nothing here knows what a case is testing. A stub records its own name and
//! argv and then does exactly what the case told it to; the candidate never sees
//! a difference between this and a real host except for the network.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub(crate) const SCHEMA: &str = include_str!("../../schemas/result-v2.schema.json");
/// Where a POSIX shell, `sleep` and `kill` live. The stub directory always goes
/// in front of this, so no case can reach a real `ssh` by accident.
pub(crate) const SYSTEM_PATH: &str = "/usr/bin:/bin";

pub(crate) struct Harness {
    root: PathBuf,
    stubs: PathBuf,
    calls: PathBuf,
    vars: BTreeMap<String, String>,
    /// A stub-only PATH, for the cases that must prove what happens when a tool
    /// is absent rather than merely failing.
    bare_path: bool,
}

#[derive(Debug)]
pub(crate) struct Call {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
}

pub(crate) struct Outcome {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) status: i32,
}

impl Harness {
    /// `case` names the scratch directory, so parallel tests never share state.
    pub(crate) fn open(case: &str) -> Result<Self, String> {
        // The scratch path is itself a plain path: a fixture that ends up in
        // JSON, or in a shell word, cannot have quote characters in it. The
        // thread name is only an isolation tag, so it is sanitised rather than
        // debug-quoted.
        let thread: String = std::thread::current()
            .name()
            .unwrap_or("main")
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
            .collect();
        let root = std::env::temp_dir().join(format!(
            "rhost-acceptance-{case}-{}-{}",
            std::process::id(),
            thread
        ));
        let _ = std::fs::remove_dir_all(&root);
        for directory in ["home/.ssh", "cache", "state", "stubs", "tmp"] {
            std::fs::create_dir_all(root.join(directory))
                .map_err(|error| fail("prepare scratch root", error))?;
        }
        let harness = Self {
            stubs: root.join("stubs"),
            calls: root.join("calls"),
            root,
            vars: BTreeMap::new(),
            bare_path: false,
        };
        // No acceptance case may fall through to the developer's real network
        // tools merely because it forgot to install its intended stand-in.
        for program in ["ssh", "scp", "rsync"] {
            harness.stub(
                program,
                "printf 'acceptance harness: unexpected external tool\\n' >&2\nexit 97",
            )?;
        }
        // A wildcard-only config: real enough to exercise discovery, and it must
        // never be reported as a host.
        harness.write(
            "home/.ssh/config",
            "Host *\n  ServerAliveInterval 30\n",
            None,
        )?;
        Ok(harness)
    }

    pub(crate) fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    pub(crate) fn write(
        &self,
        relative: &str,
        body: &str,
        mode: Option<u32>,
    ) -> Result<(), String> {
        let target = self.path(relative);
        std::fs::write(&target, body).map_err(|error| fail("write fixture", error))?;
        if let Some(bits) = mode {
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(bits))
                .map_err(|error| fail("chmod fixture", error))?;
        }
        Ok(())
    }

    /// Installs a stub for one external tool. Every stub records its own name
    /// and its argv, so a test can assert what was and was not attempted.
    pub(crate) fn stub(&self, program: &str, body: &str) -> Result<(), String> {
        // One record per invocation, one line per argument: a space-joined log
        // would lose the boundary this suite has to assert (EXEC-001).
        let script = format!(
            "#!/bin/sh\nprintf 'program {program}\\n' >> \"$RHOST_TEST_CALLS\"\nfor argument in \"$@\"; do printf 'arg %s\\n' \"$argument\" >> \"$RHOST_TEST_CALLS\"; done\n{body}\n"
        );
        let target = self.stubs.join(program);
        std::fs::write(&target, script).map_err(|error| fail("write stub", error))?;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| fail("chmod stub", error))?;
        Ok(())
    }

    pub(crate) fn env(mut self, key: &str, value: &str) -> Self {
        self.vars.insert(key.to_string(), value.to_string());
        self
    }

    /// Leaves the stubs as the *only* PATH, so a tool that is not stubbed is
    /// genuinely absent rather than found in the system directories.
    pub(crate) fn stubs_only_path(mut self) -> Self {
        for program in ["ssh", "scp", "rsync"] {
            let _ = std::fs::remove_file(self.stubs.join(program));
        }
        self.bare_path = true;
        self
    }

    /// Spawns the candidate through `env` with PATH as an assignment, so PATH is
    /// genuinely the candidate's own process environment. A bare
    /// `Command::env("PATH", ..)` would leave the candidate resolving `ssh`
    /// against this test process's PATH, and a hermetic test would quietly run
    /// the real OpenSSH client.
    pub(crate) fn command(&self, args: &[&str]) -> Result<Command, String> {
        let mut command = Command::new("/usr/bin/env");
        let path = if self.bare_path {
            self.stubs.display().to_string()
        } else {
            format!("{}:{}", self.stubs.display(), SYSTEM_PATH)
        };
        command
            .arg(format!("PATH={path}"))
            .arg(env!("CARGO_BIN_EXE_rhost"));
        command
            .current_dir(&self.root)
            .env("HOME", self.path("home"))
            .env("RHOST_CACHE_DIR", self.path("cache"))
            .env("RHOST_STATE_DIR", self.path("state"))
            .env("TMPDIR", self.path("tmp"))
            .env("RHOST_TEST_CALLS", &self.calls)
            .args(args)
            .stdin(Stdio::null());
        for (key, value) in &self.vars {
            command.env(key, value);
        }
        Ok(command)
    }

    /// One recorded invocation of an external tool.
    pub(crate) fn calls(&self) -> Vec<Call> {
        let raw = std::fs::read_to_string(&self.calls).unwrap_or_default();
        let mut calls: Vec<Call> = Vec::new();
        for line in raw.lines() {
            if let Some(program) = line.strip_prefix("program ") {
                calls.push(Call {
                    program: program.to_string(),
                    args: Vec::new(),
                });
            } else if let Some(argument) = line.strip_prefix("arg ") {
                if let Some(current) = calls.last_mut() {
                    current.args.push(argument.to_string());
                }
            }
        }
        calls
    }

    /// The recorded argv joined for substring assertions.
    pub(crate) fn calls_text(&self) -> String {
        self.calls()
            .iter()
            .map(|call| format!("{} {}", call.program, call.args.join(" ")))
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Later stub installs overwrite the script, not the log, so the log still
    /// names the tool each invocation went to.
    pub(crate) fn tool_calls(&self, program: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|call| call.program == program)
            .collect()
    }

    pub(crate) fn assert_no_remote_tool(&self, label: &str) -> Result<(), String> {
        let calls = self.calls_text();
        if calls.is_empty() {
            return Ok(());
        }
        Err(format!("input reached a remote tool ({label}): {calls}"))
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn fail(context: &str, detail: impl std::fmt::Display) -> String {
    format!("{context}: {detail}")
}

pub(crate) fn run(harness: &Harness, args: &[&str]) -> Result<Outcome, String> {
    let output = harness.command(args).and_then(|mut command| {
        command
            .output()
            .map_err(|error| fail("run candidate", error))
    })?;
    Ok(Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    })
}

/// Runs one command that must answer with a schema-valid envelope. Parsing the
/// whole trimmed stdout as a single value is what enforces "exactly one document
/// and no prose" (WIRE-002).
pub(crate) fn envelope(
    harness: &Harness,
    args: &[&str],
) -> Result<(Outcome, serde_json::Value), String> {
    let outcome = run(harness, args)?;
    let value: serde_json::Value =
        serde_json::from_str(outcome.stdout.trim()).map_err(|error| {
            fail(
                "stdout is not exactly one JSON document",
                format!(
                    "{error} stdout={:?} stderr={:?} exit={}",
                    outcome.stdout, outcome.stderr, outcome.status
                ),
            )
        })?;
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA).map_err(|error| fail("load schema", error))?;
    let validator =
        jsonschema::validator_for(&schema).map_err(|error| fail("compile schema", error))?;
    if !validator.is_valid(&value) {
        return Err(format!("{args:?} produced a rejected envelope: {value}"));
    }
    if value["ok"].as_bool().unwrap_or(false) != value["error"].is_null() {
        return Err(format!("{args:?} disagrees between ok and error: {value}"));
    }
    Ok((outcome, value))
}

pub(crate) fn want_code(value: &serde_json::Value, expected: &str) -> Result<(), String> {
    let got = value
        .pointer("/error/code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("<none>");
    if got == expected {
        Ok(())
    } else {
        Err(format!("error.code={got} want {expected}: {value}"))
    }
}

pub(crate) fn operation(value: &serde_json::Value) -> String {
    value
        .get("operation")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Whether this machine can run the embedded helper the way a remote host
/// would. `make check` must not require python3, so the editing tests assert the
/// dependency answer when it is absent instead of passing silently.
pub(crate) fn python3_available() -> bool {
    std::process::Command::new("/usr/bin/env")
        .args(["sh", "-c", "command -v python3 >/dev/null 2>&1"])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
