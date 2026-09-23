//! The dedicated OpenSSH master one tunnel owns.
//!
//! Every other command rides the one shared `%C` master. A tunnel deliberately
//! does not: `tunnel close` has to stop exactly one forward without dropping the
//! multiplexed connection that `exec` and `session` traffic is using. So a
//! tunnel's master is started here, with its own `ControlPath`, and `alive` is
//! only ever the answer to a control request that master actually gave.

use super::record::Fault;
use crate::transport::run as run_transport;
use crate::transport::{Recorder, Run, RunFailure, Spec, StdinSource};
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

/// How long ssh may take to authenticate, bind the forward and hand it to a
/// background master. `-f` returns once that happened; this is the budget for
/// getting there.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// One `ssh -O` request is a local control-protocol exchange with a master on
/// this machine: a master that cannot answer in this long is not worth waiting
/// for.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(15);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

/// What one control exchange established.
pub enum Control {
    /// The master answered: this forward exists.
    Answered,
    /// The socket is not there. This is the only observation that proves the
    /// recorded master is gone.
    SocketAbsent,
    /// Something is there and did not answer. That is not evidence of death, and
    /// rhost must not act as though it were.
    Uncertain(String),
}

pub struct Master<'a> {
    pub ssh_bin: &'a str,
    /// rhost's ordinary OpenSSH options, passed *after* the tunnel's own.
    /// OpenSSH keeps the first value it is given for a parameter, so the tunnel's
    /// `ControlPath` and `ControlMaster` win over the shared socket and the
    /// multiplexed auto-master. That is what makes this connection independent of
    /// every other command.
    pub options: &'a [String],
}

impl Master<'_> {
    /// Starts the dedicated master carrying exactly one forward, and refuses to
    /// return until it has answered a `check`.
    ///
    /// `ExitOnForwardFailure=yes` is what makes a refused bind an error here
    /// instead of a master that is up with no forward; the `check` afterwards is
    /// what makes `alive` an observation rather than a hope that ssh returned
    /// quickly enough. The caller owns cleanup using the record it published
    /// before invoking this method.
    pub fn start(
        &self,
        socket: &Path,
        kind_flag: &str,
        forward: &str,
        host: &str,
    ) -> Result<(), Fault> {
        let mut args: Vec<String> = vec![
            "-o".into(),
            format!("ControlPath={}", socket.display()),
            "-o".into(),
            "ControlMaster=yes".into(),
            "-o".into(),
            "ControlPersist=yes".into(),
            "-o".into(),
            "ExitOnForwardFailure=yes".into(),
            "-o".into(),
            "ServerAliveInterval=15".into(),
            "-o".into(),
            "ServerAliveCountMax=3".into(),
        ];
        args.extend(self.options.iter().cloned());
        // `-f` backgrounds the master once the forward is up, `-N` runs no remote
        // command: this connection exists only to hold the forward.
        args.push("-fNT".into());
        args.push(kind_flag.into());
        args.push(forward.into());
        args.push("--".into());
        args.push(host.into());
        let (diagnostic, run) = self.capture(&args, START_TIMEOUT);
        if run.timed_out {
            return Err(Fault::Uncertain(format!(
                "tunnel startup timed out: {}",
                complaint(&run, &diagnostic)
            )));
        }
        if run.exit != Some(0) {
            return Err(Fault::Transport(format!(
                "cannot open tunnel: {}",
                complaint(&run, &diagnostic)
            )));
        }
        match self.check(socket, host) {
            Control::Answered => Ok(()),
            other => Err(Fault::Transport(format!(
                "tunnel started but its OpenSSH master is not answering: {}",
                reason(&other)
            ))),
        }
    }

    pub fn check(&self, socket: &Path, host: &str) -> Control {
        self.control(socket, host, "check")
    }

    /// Asks one master to exit. Only that tunnel's socket is addressed, so the
    /// shared connection and every other tunnel keep running.
    pub fn exit(&self, socket: &Path, host: &str) -> Control {
        self.control(socket, host, "exit")
    }

    /// An accepted exit request is followed by a bounded check for socket
    /// removal. Until then another process could still reach the master.
    pub fn stop(&self, socket: &Path, host: &str) -> Result<(), String> {
        match self.exit(socket, host) {
            Control::SocketAbsent => return Ok(()),
            Control::Uncertain(reason) => return Err(reason),
            Control::Answered => {}
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        loop {
            match std::fs::symlink_metadata(socket) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(format!("inspect {}: {error}", socket.display())),
                Ok(_) if Instant::now() >= deadline => {
                    return Err(format!(
                        "OpenSSH accepted exit but control socket {} remains",
                        socket.display()
                    ));
                }
                Ok(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    fn control(&self, socket: &Path, host: &str, action: &str) -> Control {
        match std::fs::symlink_metadata(socket) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Control::SocketAbsent;
            }
            Err(error) => {
                return Control::Uncertain(format!("inspect {}: {error}", socket.display()));
            }
        }
        let args: Vec<String> = vec![
            "-S".into(),
            socket.display().to_string(),
            "-O".into(),
            action.into(),
            "--".into(),
            host.into(),
        ];
        let (diagnostic, run) = self.capture(&args, CONTROL_TIMEOUT);
        if run.exit == Some(0) {
            Control::Answered
        } else {
            Control::Uncertain(format!("ssh -O {action}: {}", complaint(&run, &diagnostic)))
        }
    }

    /// Runs one ssh invocation and reads both streams together: OpenSSH writes
    /// its diagnostics and its control answers on stderr.
    fn capture(&self, args: &[String], timeout: Duration) -> (String, Run) {
        let mut stdout = Recorder::new(0);
        let mut stderr = Recorder::new(0);
        let run = run_transport(
            &Spec {
                program: self.ssh_bin,
                args,
                stdin: StdinSource::Closed,
                deadline: Some(Instant::now() + timeout),
                cancelled: &never,
                // The master must not be killed as a group: it deliberately
                // leaves a background process behind when `-f` returns.
                group: false,
            },
            &mut stdout,
            &mut stderr,
        );
        let mut combined = stdout.body();
        combined.extend_from_slice(&stderr.body());
        (String::from_utf8_lossy(&combined).trim().to_string(), run)
    }
}

fn never() -> bool {
    false
}

/// Why a control exchange produced no answer, in OpenSSH's own words where it
/// gave any. The words are the actionable part: a bind the remote sshd refused
/// will be refused again.
fn reason(control: &Control) -> String {
    match control {
        Control::Uncertain(reason) => reason.clone(),
        Control::SocketAbsent => "its control socket is absent".to_string(),
        Control::Answered => "it answered".to_string(),
    }
}

fn complaint(run: &Run, diagnostic: &str) -> String {
    if !diagnostic.is_empty() {
        return diagnostic.to_string();
    }
    if let Some(RunFailure::Spawn(error)) = &run.failure {
        return format!("cannot run ssh: {error}");
    }
    if run.timed_out {
        return "ssh did not finish in time".to_string();
    }
    match run.exit {
        Some(code) => format!("ssh exited with status {code} and said nothing"),
        None => "ssh was stopped without reporting a status".to_string(),
    }
}
