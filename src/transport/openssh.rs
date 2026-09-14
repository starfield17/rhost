//! The system OpenSSH client, driven as a child process.
//!
//! OpenSSH owns resolution, authentication and host-key policy (AGENTS.md §5):
//! rhost passes options, never a password, and never disables host-key
//! checking. ControlMaster plus ControlPersist plus a `%C` ControlPath give
//! transport reuse that outlives any single rhost process, without a daemon.
use super::process::{self, Recorder, Run, Spec, StdinSource, Stream};
use crate::config;
use std::time::{Duration, Instant};

/// Connection reuse outlives one invocation, but not forever.
const CONTROL_PERSIST: &str = "15m";
const CONNECT_TIMEOUT_SECS: u64 = 15;
/// `ssh -O check` and `ssh -O stop` are local control-protocol exchanges; a
/// master that cannot answer in this long is not worth waiting for.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Config {
    pub ssh_bin: String,
    pub control_path: String,
    pub control_persist: String,
    pub connect_timeout: Duration,
    pub batch_mode: bool,
    pub log_level: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ssh_bin: "ssh".into(),
            control_path: config::control_path(),
            control_persist: CONTROL_PERSIST.into(),
            connect_timeout: Duration::from_secs(CONNECT_TIMEOUT_SECS),
            batch_mode: true,
            // OpenSSH suppresses its own "Connection closed by ..." diagnostics
            // at LogLevel=ERROR, which would otherwise leave rhost with an empty
            // stderr to classify. RHOST_SSH_LOG_LEVEL raises it for diagnosis.
            log_level: std::env::var("RHOST_SSH_LOG_LEVEL").unwrap_or_else(|_| "ERROR".into()),
        }
    }
}

pub struct Client {
    cfg: Config,
}

fn never() -> bool {
    false
}

impl Client {
    pub fn new(cfg: Config) -> Self {
        Self { cfg }
    }

    pub fn control_path(&self) -> &str {
        &self.cfg.control_path
    }

    /// The OpenSSH program every command under this client runs. A dedicated
    /// tunnel master is started by the same client, so it must be the same binary
    /// rather than a second, private resolution of "ssh".
    pub fn ssh_bin(&self) -> &str {
        &self.cfg.ssh_bin
    }

    /// The OpenSSH options rhost passes to ssh, in argv form.
    ///
    /// No caller may build its own ssh argument list from scratch: transport
    /// reuse depends on these being the only ones (AGENTS.md §5). `scp` takes
    /// them directly and `rsync` carries them inside a single `-e` string.
    pub fn options(&self, fresh: bool) -> Vec<String> {
        let (master, persist, path) = if fresh {
            ("no", "no", "none")
        } else {
            (
                "auto",
                self.cfg.control_persist.as_str(),
                self.cfg.control_path.as_str(),
            )
        };
        let mut options = vec![
            "-o".to_string(),
            format!("ControlMaster={master}"),
            "-o".to_string(),
            format!("ControlPersist={persist}"),
            "-o".to_string(),
            format!("ControlPath={path}"),
            "-o".to_string(),
            format!("LogLevel={}", self.cfg.log_level),
            "-o".to_string(),
            format!(
                "ConnectTimeout={}",
                self.cfg.connect_timeout.as_secs().max(1)
            ),
        ];
        if self.cfg.batch_mode {
            // Never prompt: agents run non-interactively. Authentication must
            // come from keys or ssh-agent.
            options.push("-o".to_string());
            options.push("BatchMode=yes".to_string());
        }
        options
    }

    pub fn ssh_options(&self) -> Vec<String> {
        self.options(false)
    }

    fn exec_argv(&self, target: &str, remote_command: &str, fresh: bool) -> Vec<String> {
        let mut args = self.options(fresh);
        // -T: no pseudo-terminal. The remote command string is one argument, so
        // exactly one shell program crosses the boundary (EXEC-001).
        args.push("-T".to_string());
        args.push("--".to_string());
        args.push(target.to_string());
        args.push(remote_command.to_string());
        args
    }

    /// Runs one remote command and keeps its output in bounded memory.
    pub fn run_captured(
        &self,
        target: &str,
        remote_command: &str,
        timeout: Duration,
        max_output_bytes: usize,
        stdin: Option<&[u8]>,
        fresh: bool,
    ) -> Result<Captured, config::UnsafeLocalState> {
        if !fresh {
            config::ensure_control_dir()?;
        }
        let args = self.exec_argv(target, remote_command, fresh);
        let mut stdout = Recorder::new(max_output_bytes);
        let mut stderr = Recorder::new(max_output_bytes);
        let run = process::run(
            &Spec {
                program: &self.cfg.ssh_bin,
                args: &args,
                stdin: match stdin {
                    Some(payload) => StdinSource::Payload(payload),
                    None => StdinSource::Closed,
                },
                deadline: Some(Instant::now() + timeout),
                cancelled: &never,
                group: false,
            },
            &mut stdout,
            &mut stderr,
        );
        Ok(Captured {
            stdout: stdout.body(),
            stderr: stderr.body(),
            stdout_bytes: stdout.total(),
            stderr_bytes: stderr.total(),
            run,
        })
    }

    /// Runs one remote command while its streams are forwarded as they are
    /// produced, so bytes are observable before the remote process exits.
    pub fn run_streams(
        &self,
        call: &Streams<'_>,
        stdout: &mut dyn Stream,
        stderr: &mut dyn Stream,
    ) -> Result<Run, config::UnsafeLocalState> {
        if !call.fresh {
            config::ensure_control_dir()?;
        }
        let args = self.exec_argv(call.target, call.remote_command, call.fresh);
        Ok(process::run(
            &Spec {
                program: &self.cfg.ssh_bin,
                args: &args,
                stdin: call.stdin,
                deadline: call.timeout.map(|limit| Instant::now() + limit),
                cancelled: call.cancelled,
                group: false,
            },
            stdout,
            stderr,
        ))
    }

    /// Asks OpenSSH whether the shared master is answering. It never creates a
    /// master, so a caller cannot claim reuse from an argv that merely carried a
    /// ControlPath option.
    pub fn connection_status(&self, target: &str) -> ConnectionStatus {
        let mut args = self.options(false);
        args.push("-O".to_string());
        args.push("check".to_string());
        args.push("--".to_string());
        args.push(target.to_string());
        let (diagnostic, exit) = self.control_probe(&args);
        ConnectionStatus::inspect(self.cfg.control_path.clone(), &diagnostic, exit)
    }

    /// Stops a shared master accepting new requests. `stop`, never `exit`, so
    /// channels already accepted keep running (SSH-002).
    ///
    /// The status always describes what the reset acted on, even when the reset
    /// failed: a caller has to be able to tell "there was no master" from "I
    /// could not tell" (SSH-002).
    pub fn reset_connection(&self, target: &str) -> Reset {
        let status = self.connection_status(target);
        match status.master_status {
            MasterStatus::Absent => Reset {
                status,
                error: None,
            },
            MasterStatus::Unknown => Reset {
                error: Some(status.diagnostic.clone()),
                status,
            },
            MasterStatus::Alive => {
                let mut args = self.options(false);
                args.push("-O".to_string());
                args.push("stop".to_string());
                args.push("--".to_string());
                args.push(target.to_string());
                let (diagnostic, exit) = self.control_probe(&args);
                let mut status = status;
                status.diagnostic = diagnostic.clone();
                if exit == Some(0) {
                    // `master_status` reports the state the reset acted on. A
                    // stop only withdraws the listening socket, so channels
                    // already accepted keep running (SSH-002).
                    status.stopped = true;
                    Reset {
                        status,
                        error: None,
                    }
                } else {
                    // The stop did not land, so the master's state is no longer
                    // known to us; saying so is the whole claim.
                    status.master_status = MasterStatus::Unknown;
                    Reset {
                        error: Some(if diagnostic.is_empty() {
                            format!("ssh -O stop failed (exit {exit:?})")
                        } else {
                            diagnostic
                        }),
                        status,
                    }
                }
            }
        }
    }

    /// One control-protocol exchange: OpenSSH writes its answer on stderr, so
    /// both streams are read together.
    fn control_probe(&self, args: &[String]) -> (String, Option<i32>) {
        let mut stdout = Recorder::new(0);
        let mut stderr = Recorder::new(0);
        let run = process::run(
            &Spec {
                program: &self.cfg.ssh_bin,
                args,
                stdin: StdinSource::Closed,
                deadline: Some(Instant::now() + CONTROL_TIMEOUT),
                cancelled: &never,
                group: false,
            },
            &mut stdout,
            &mut stderr,
        );
        let mut combined = stdout.body();
        combined.extend_from_slice(&stderr.body());
        (
            String::from_utf8_lossy(&combined).trim().to_string(),
            run.exit,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterStatus {
    Alive,
    Absent,
    Unknown,
}

impl MasterStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alive => "alive",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        }
    }
}

/// Evidence returned by OpenSSH's multiplexing control protocol. `control_path`
/// is the template passed to OpenSSH; `master_pid` is present only when OpenSSH
/// reported it explicitly.
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    pub master_status: MasterStatus,
    pub control_path: String,
    pub master_pid: Option<u32>,
    pub diagnostic: String,
    pub stopped: bool,
}

const PID_MARK: &str = "master running (pid=";

impl ConnectionStatus {
    fn inspect(control_path: String, combined: &str, exit: Option<i32>) -> Self {
        let lower = combined.to_lowercase();
        let (master_status, master_pid) = if exit == Some(0) {
            (MasterStatus::Alive, parse_master_pid(combined))
        } else {
            let absent = lower.contains("no such file or directory")
                || lower.contains("no control master")
                || (lower.contains("control socket connect")
                    && lower.contains("connection refused"));
            (
                if absent {
                    MasterStatus::Absent
                } else {
                    MasterStatus::Unknown
                },
                None,
            )
        };
        Self {
            master_status,
            control_path,
            master_pid,
            diagnostic: combined.trim().to_string(),
            stopped: false,
        }
    }
}

fn parse_master_pid(diagnostic: &str) -> Option<u32> {
    let start = diagnostic.to_lowercase().find(PID_MARK)? + PID_MARK.len();
    let digits = diagnostic[start..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    if digits == 0 {
        return None;
    }
    diagnostic[start..start + digits].parse().ok()
}

/// One streamed run: what to execute, and how the caller observes it.
///
/// A struct rather than a long argument list, because the three stream
/// arguments (`stdin`, `cancelled`, and the two sinks) all describe the same
/// thing — how this invocation is watched — and a caller must not be able to
/// transpose them.
pub struct Streams<'a> {
    pub target: &'a str,
    pub remote_command: &'a str,
    /// `None` means no deadline: an ordinary foreground run has one (EXEC-002).
    pub timeout: Option<Duration>,
    pub stdin: StdinSource<'a>,
    pub fresh: bool,
    /// Polled while the child runs; a run a human asked to stop must stop.
    pub cancelled: &'a dyn Fn() -> bool,
}

/// A reset attempt: the state it acted on, plus why it did not land.
pub struct Reset {
    pub status: ConnectionStatus,
    pub error: Option<String>,
}

/// The raw outcome of one ssh invocation, with counts of the *source* bytes.
pub struct Captured {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub run: Run,
}
