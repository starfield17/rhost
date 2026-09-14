//! The child process and the loop that owns it.
//!
//! One main loop decides when a run is over; the reader threads only move bytes.
//! The loop polls for the exit status and for cancellation, and stops the whole
//! process group when the run is stopped, so a tool that forked a wrapper cannot
//! keep the pipes open after the tool itself is gone.

use super::capture::Stream;
use std::io::{self, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{SyncSender, TryRecvError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long to keep draining after the child is gone. A local descendant that
/// inherited a pipe can otherwise hold the run open long after ssh exited.
pub const DRAIN_GRACE: Duration = Duration::from_millis(2_000);

/// Poll interval for the child's exit status and for cancellation requests.
const POLL: Duration = Duration::from_millis(10);

/// Size of one chunk moved from a reader thread to the main loop.
const CHUNK: usize = 64 * 1024;

/// Chunks a reader may queue before it stops consuming its pipe. Back-pressure
/// is what keeps a runaway command from turning the CLI's memory into the size
/// of its output.
const QUEUE: usize = 64;

/// How the child's stdin is fed.
#[derive(Clone, Copy)]
pub enum StdinSource<'a> {
    /// The child inherits this process' stdin, so a remote command can read it
    /// and sees EOF when the caller's own input closes.
    Inherit,
    /// A fixed payload written by a detached feeder, so a large payload can
    /// never deadlock the loop that owns the deadline.
    Payload(&'a [u8]),
    /// Nothing to read: the child's stdin is /dev/null.
    Closed,
}

pub struct Spec<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub stdin: StdinSource<'a>,
    /// `None` means no execution deadline: wait until the child finishes or the
    /// caller cancels.
    pub deadline: Option<Instant>,
    /// Polled between waits: a run a human asked to stop must stop.
    pub cancelled: &'a dyn Fn() -> bool,
    /// Start the child as its own process group leader and stop that whole group
    /// when the run is stopped. A tool that forked a wrapper — `scp` running a
    /// shell function, `rsync` starting a helper — otherwise keeps the pipes
    /// open after the tool itself is gone, which turns a deadline into an
    /// abandoned transfer.
    pub group: bool,
}

/// Why a run stopped early. The two causes are never interchangeable: a child
/// that never started is a connectivity question, while a stream that stopped
/// accepting bytes is a delivery failure whose remote side may be fine.
#[derive(Debug)]
pub enum RunFailure {
    Spawn(io::Error),
    Stream(io::Error),
}

pub struct Run {
    /// `None` means no exit status was reported: killed locally, or signalled.
    pub exit: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub duration: Duration,
    pub failure: Option<RunFailure>,
}

enum Chunk {
    Data(Vec<u8>),
    Failed(String),
}

fn pump<R: Read>(mut pipe: R, sender: SyncSender<Chunk>) {
    let mut buffer = vec![0u8; CHUNK];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                let mut payload = Vec::with_capacity(read);
                payload.extend_from_slice(&buffer[..read]);
                if sender.send(Chunk::Data(payload)).is_err() {
                    break;
                }
            }
            Err(ref interrupted) if interrupted.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = sender.send(Chunk::Failed(error.to_string()));
                break;
            }
        }
    }
}

/// Stops the child and, when the run owns a process group, everything that
/// inherited its pipes. The external `kill` syntax below is shared by the BSD
/// and GNU implementations; shell builtins disagree about `--` before a
/// negative process-group id. If it cannot run, the per-process kill below
/// still stops the tool itself.
fn stop(child: &mut std::process::Child, group: bool) {
    if group {
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg("--")
            .arg(format!("-{}", child.id()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// Moves everything one reader has already produced into its sink.
fn pump_stream(
    slot: &mut Option<std::sync::mpsc::Receiver<Chunk>>,
    sink: &mut dyn Stream,
    failure: &mut Option<RunFailure>,
) {
    let Some(receiver) = slot.as_ref() else {
        return;
    };
    loop {
        match receiver.try_recv() {
            Ok(Chunk::Data(bytes)) => {
                if let Err(error) = sink.feed(&bytes) {
                    if failure.is_none() {
                        *failure = Some(RunFailure::Stream(error));
                    }
                    return;
                }
            }
            Ok(Chunk::Failed(text)) => {
                if failure.is_none() {
                    *failure = Some(RunFailure::Stream(io::Error::other(text)));
                }
                return;
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                *slot = None;
                return;
            }
        }
    }
}

/// Runs one local process to completion, deadline, cancellation, or failure,
/// routing its two output streams into the caller's sinks.
pub fn run(spec: &Spec<'_>, stdout: &mut dyn Stream, stderr: &mut dyn Stream) -> Run {
    let start = Instant::now();
    let mut command = Command::new(spec.program);
    command
        .args(spec.args)
        .stdin(match spec.stdin {
            StdinSource::Inherit => Stdio::inherit(),
            StdinSource::Payload(_) => Stdio::piped(),
            StdinSource::Closed => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if spec.group {
        // Its own group means a signal aimed at the tool cannot reach the CLI
        // that started it, and the group can be stopped as one thing.
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return settle(start, None, false, false, Some(RunFailure::Spawn(error))),
    };
    // A dropped child is left running: this loop owns the child's lifetime, and
    // walking away must never silently kill a remote command that was already
    // submitted.

    if let (StdinSource::Payload(payload), Some(mut pipe)) = (spec.stdin, child.stdin.take()) {
        let payload = payload.to_vec();
        // Detached on purpose: the loop owns the deadline, and a child that
        // never reads must not become a reason to wait forever.
        thread::spawn(move || {
            let _ = pipe.write_all(&payload);
            let _ = pipe.flush();
        });
    }

    let (out_sender, out_receiver) = sync_channel::<Chunk>(QUEUE);
    let (err_sender, err_receiver) = sync_channel::<Chunk>(QUEUE);
    let mut readers: Vec<JoinHandle<()>> = Vec::new();
    if let Some(pipe) = child.stdout.take() {
        readers.push(thread::spawn(move || pump(pipe, out_sender)));
    }
    if let Some(pipe) = child.stderr.take() {
        readers.push(thread::spawn(move || pump(pipe, err_sender)));
    }
    let mut out_slot = Some(out_receiver);
    let mut err_slot = Some(err_receiver);

    let mut exit: Option<i32> = None;
    // A child killed by a signal reports no exit code, and "no code" is not the
    // same fact as "not finished". Reaping has to be tracked on its own, or the
    // loop waits out the drain grace after every stop it asked for.
    let mut reaped = false;
    let mut timed_out = false;
    let mut cancelled = false;
    let mut failure: Option<RunFailure> = None;
    let mut stop_deadline: Option<Instant> = None;
    let mut killing = false;

    loop {
        pump_stream(&mut out_slot, stdout, &mut failure);
        pump_stream(&mut err_slot, stderr, &mut failure);
        let stopped = failure.is_some();

        if exit.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exit = status.code();
                    reaped = true;
                    stop_deadline = stop_deadline.or(Some(Instant::now() + DRAIN_GRACE));
                }
                Ok(None) => {}
                Err(error) => {
                    if failure.is_none() {
                        failure = Some(RunFailure::Spawn(error));
                    }
                    exit = None;
                    stop_deadline = Some(Instant::now());
                }
            }
        }

        if exit.is_none() && !killing && !stopped {
            if spec
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                timed_out = true;
            } else if (spec.cancelled)() {
                cancelled = true;
            }
            if timed_out || cancelled {
                killing = true;
                stop(&mut child, spec.group);
                stop_deadline = Some(Instant::now() + DRAIN_GRACE);
            }
        }
        if stopped && !killing {
            // Output that cannot be delivered must not keep a remote command
            // running behind a dead pipe.
            killing = true;
            stop(&mut child, spec.group);
            stop_deadline = Some(Instant::now() + DRAIN_GRACE);
        }

        if out_slot.is_none() && err_slot.is_none() && reaped {
            break;
        }
        if stop_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            break;
        }
        thread::sleep(POLL);
    }

    if let Err(error) = stdout.finish() {
        if failure.is_none() {
            failure = Some(RunFailure::Stream(error));
        }
    }
    if let Err(error) = stderr.finish() {
        if failure.is_none() {
            failure = Some(RunFailure::Stream(error));
        }
    }
    let _ = readers;
    settle(start, exit, timed_out, cancelled, failure)
}

fn settle(
    start: Instant,
    exit: Option<i32>,
    timed_out: bool,
    cancelled: bool,
    failure: Option<RunFailure>,
) -> Run {
    Run {
        exit,
        timed_out,
        cancelled,
        duration: start.elapsed(),
        failure,
    }
}
