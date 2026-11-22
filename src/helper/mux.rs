//! One verified SSH channel with bounded, independently routed observations.
mod bootstrap;
use super::{Helper, Writer, codec};
use crate::{domain::Recovery, remote::Error};
use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::process::Child;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, RecvTimeoutError, SyncSender},
};
use std::time::{Duration, Instant};

pub const FOLLOW: u32 = 11;
pub const DATA: u32 = 12;
pub const COMPLETION: u32 = 13;
const CANCEL: u32 = 14;
pub(super) const FILES: u32 = 15;
const REPLY: u32 = 0x80000000;
const IN_FLIGHT: usize = 24;
const HOSTS: usize = 8;
const HEADER: usize = 20;
const INTERVAL: Duration = Duration::from_millis(20);

struct Frame {
    id: u64,
    kind: u32,
    body: Vec<u8>,
}

impl Frame {
    fn encode(id: u64, kind: u32, body: Vec<u8>) -> Result<Vec<u8>, Error> {
        if id == 0 || body.len() > codec::LIMIT {
            return Err(codec::protocol("invalid RH02 request frame"));
        }
        let mut wire = Vec::with_capacity(HEADER + body.len());
        wire.extend(b"RH02");
        wire.extend(id.to_be_bytes());
        wire.extend(kind.to_be_bytes());
        wire.extend((body.len() as u32).to_be_bytes());
        wire.extend(body);
        Ok(wire)
    }

    fn read(input: &mut impl Read) -> Result<Self, Error> {
        let mut header = [0; HEADER];
        input.read_exact(&mut header).map_err(|_| unanswered())?;
        if &header[..4] != b"RH02" {
            return Err(codec::protocol("invalid RH02 reply header"));
        }
        let mut fields = codec::Reader::new(&header[4..]);
        let id = fields.long()?;
        let reply = fields.int()?;
        let size = fields.int()? as usize;
        let kind = reply & !REPLY;
        if id == 0
            || reply & REPLY == 0
            || (!(2..=COMPLETION).contains(&kind) && kind != FILES)
            || size > codec::LIMIT
            || (kind == DATA && size > 13 + 16384)
        {
            return Err(codec::protocol("invalid RH02 reply binding or size"));
        }
        let mut body = vec![0; size];
        input.read_exact(&mut body).map_err(|_| unanswered())?;
        Ok(Self { id, kind, body })
    }
}

fn unanswered() -> Error {
    Error::new(
        "REMOTE_EXECUTION_UNKNOWN",
        "verified RH02 answer is unavailable",
    )
}

fn cancelled() -> Error {
    Error::new(
        "REMOTE_COMMAND_CANCELLED",
        "local RH02 observation cancelled",
    )
}

fn before(error: Error) -> Error {
    error.with_recovery(Recovery::not_dispatched())
}

struct Routing {
    pending: Mutex<HashMap<u64, (u32, SyncSender<Frame>)>>,
    alive: AtomicBool,
}

impl Routing {
    fn close(&self) {
        self.alive.store(false, Ordering::Release);
        if let Ok(mut pending) = self.pending.lock() {
            pending.clear();
        }
    }

    fn read(&self, mut input: impl Read) {
        while let Ok(frame) = Frame::read(&mut input) {
            let Ok(mut pending) = self.pending.lock() else {
                break;
            };
            let id = frame.id;
            let Some((kind, reply)) = pending.get(&id) else {
                continue;
            };
            let last = frame.kind == *kind;
            if !last && !(*kind == FOLLOW && matches!(frame.kind, DATA | COMPLETION)) {
                break;
            }
            if reply.try_send(frame).is_err() || last {
                pending.remove(&id);
            }
        }
        self.close();
    }
}

struct Bootstrap {
    ready: mpsc::Receiver<Result<Vec<u8>, Error>>,
    stderr: mpsc::Receiver<Vec<u8>>,
}

struct Channel {
    child: Mutex<Child>,
    writer: SyncSender<Vec<u8>>,
    routing: Arc<Routing>,
    next: AtomicU64,
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Channel {
    fn stop(&self) {
        self.routing.close();
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn new(mut child: Child, marker: String) -> Result<(Self, Bootstrap), Error> {
        let missing_pipe = || before(codec::protocol("SSH channel is missing a pipe"));
        let output = child.stdout.take().ok_or_else(missing_pipe)?;
        let mut input = child.stdin.take().ok_or_else(missing_pipe)?;
        let mut stderr = child.stderr.take().ok_or_else(missing_pipe)?;
        let routing = Arc::new(Routing {
            pending: Mutex::new(HashMap::new()),
            alive: AtomicBool::new(true),
        });
        let (send, receive) = mpsc::sync_channel::<Vec<u8>>(IN_FLIGHT);
        let channel = Self {
            child: Mutex::new(child),
            writer: send,
            routing: routing.clone(),
            next: AtomicU64::new(1),
        };
        let read = routing.clone();
        let (ready, verified) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .spawn(move || {
                let mut output = BufReader::new(output);
                let probe = bootstrap::handshake(&mut output, &marker);
                let ok = probe.is_ok();
                if ready.send(probe).is_ok() && ok {
                    read.read(output);
                } else {
                    read.close();
                }
            })
            .map_err(|e| before(Error::new("INTERNAL", e.to_string())))?;
        std::thread::Builder::new()
            .spawn(move || {
                for frame in receive {
                    if input.write_all(&frame).is_err() {
                        break;
                    }
                }
                routing.close();
            })
            .map_err(|e| before(Error::new("INTERNAL", e.to_string())))?;
        let (diagnostic, stderr_result) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .spawn(move || {
                let mut kept = Vec::new();
                let _ = (&mut stderr).take(16384).read_to_end(&mut kept);
                let _ = std::io::copy(&mut stderr, &mut std::io::sink());
                let _ = diagnostic.send(kept);
            })
            .map_err(|e| before(Error::new("INTERNAL", e.to_string())))?;
        Ok((
            channel,
            Bootstrap {
                ready: verified,
                stderr: stderr_result,
            },
        ))
    }

    fn request(
        &self,
        kind: u32,
        body: Writer,
        wait_ms: u32,
        interrupted: &dyn Fn() -> bool,
        mut event: impl FnMut(u32, &[u8]) -> Result<(), Error>,
    ) -> Result<Vec<u8>, Error> {
        if interrupted() {
            return Err(before(cancelled()));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let wire = Frame::encode(id, kind, body.0)?;
        // FOLLOW can produce two 1 MiB streams in 16 KiB frames, plus leader
        // completion and the terminal receipt. Regular replies use one frame.
        // If small live chunks fill this queue, routing fails this request
        // rather than dropping bytes or blocking unrelated requests.
        let (send, receive) = mpsc::sync_channel(if kind == FOLLOW { 132 } else { 1 });
        let mut pending = self
            .routing
            .pending
            .lock()
            .map_err(|_| before(unanswered()))?;
        if !self.routing.alive.load(Ordering::Acquire) {
            return Err(before(unanswered()));
        }
        if pending.len() >= IN_FLIGHT {
            return Err(before(Error::new(
                "STATE_UNAVAILABLE",
                "RH02 channel is at capacity",
            )));
        }
        pending.insert(id, (kind, send));
        if self.writer.try_send(wire).is_err() {
            pending.remove(&id);
            return Err(before(unanswered()));
        }
        drop(pending);
        let deadline = Instant::now() + Duration::from_millis(u64::from(wait_ms));
        let result = loop {
            if interrupted() {
                break Err(cancelled());
            }
            match receive.recv_timeout(INTERVAL) {
                Ok(frame) if frame.kind == kind => return codec::result(&frame.body),
                Ok(frame) => {
                    if let Err(error) = event(frame.kind, &frame.body) {
                        break Err(error);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break Err(unanswered()),
                Err(RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                    break Err(Error::new("REMOTE_COMMAND_TIMEOUT", "RH02 reply timed out"));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        };
        if let Ok(mut pending) = self.routing.pending.lock() {
            pending.remove(&id);
        }
        if matches!(kind, 2 | 3 | 4 | 8 | 9 | 10 | FOLLOW)
            && let Ok(frame) = Frame::encode(id, CANCEL, Vec::new())
        {
            let _ = self.writer.try_send(frame);
        }
        result
    }
}

type Slot = Arc<Mutex<Option<Arc<Channel>>>>;

/// Process-scoped reuse for a single request server. Host and boot guard are
/// part of the key. Dropping the server closes its verified SSH channels.
#[derive(Default)]
pub(crate) struct Pool {
    slots: Mutex<HashMap<(String, String), Slot>>,
}

impl Pool {
    fn channel(&self, helper: &Helper<'_>) -> Result<Arc<Channel>, Error> {
        let key = (
            helper.host.to_owned(),
            helper
                .boot_guard
                .map_or("", |boot| boot.as_str())
                .to_owned(),
        );
        let mut slots = self.slots.lock().map_err(|_| before(unanswered()))?;
        if !slots.contains_key(&key) && slots.len() >= HOSTS {
            let idle = slots.iter().find_map(|(host, slot)| {
                (Arc::strong_count(slot) == 1
                    && slot.try_lock().ok().is_some_and(|value| {
                        value
                            .as_ref()
                            .is_none_or(|channel| Arc::strong_count(channel) == 1)
                    }))
                .then(|| host.clone())
            });
            if let Some(idle) = idle {
                slots.remove(&idle);
            } else {
                return Err(before(Error::new(
                    "STATE_UNAVAILABLE",
                    "RH02 host pool is at capacity",
                )));
            }
        }
        let slot = slots.entry(key).or_default().clone();
        drop(slots);
        let mut channel = loop {
            if (helper.cancelled)() {
                return Err(before(cancelled()));
            }
            match slot.try_lock() {
                Ok(value) => break value,
                Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(INTERVAL),
                Err(std::sync::TryLockError::Poisoned(_)) => return Err(before(unanswered())),
            }
        };
        if let Some(existing) = channel.as_ref()
            && existing.routing.alive.load(Ordering::Acquire)
        {
            return Ok(existing.clone());
        }
        let opened = Arc::new(helper.open_mux(true)?);
        *channel = Some(opened.clone());
        Ok(opened)
    }
}

impl Helper<'_> {
    pub(crate) fn mux(
        &self,
        pool: Option<&Pool>,
        kind: u32,
        body: Writer,
        wait_ms: u32,
        event: impl FnMut(u32, &[u8]) -> Result<(), Error>,
    ) -> Result<Vec<u8>, Error> {
        let channel = if let Some(pool) = pool {
            pool.channel(self)?
        } else {
            Arc::new(self.open_mux(true)?)
        };
        channel.request(
            kind,
            body,
            wait_ms.saturating_add(15000),
            self.cancelled,
            event,
        )
    }

    pub(super) fn mux_file(
        &self,
        pool: &Pool,
        request: &[u8],
        timeout: Duration,
        cap: usize,
    ) -> Result<Vec<u8>, Error> {
        let mut body = Writer::default();
        body.int(cap as u32);
        body.0.extend(request);
        let timeout = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        pool.channel(self)?
            .request(FILES, body, timeout, self.cancelled, |_, _| {
                Err(codec::protocol("unexpected file event"))
            })
            .map_err(|error| {
                if error.recovery.submission() == "not_submitted" {
                    error
                } else {
                    error.with_mutation(crate::domain::MutationEvidence::Unknown)
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frame_binds_response_to_request_and_refuses_unbounded_lengths() -> Result<(), Error> {
        let mut bytes = Frame::encode(7, DATA | REPLY, vec![0, 255])?;
        let reply = Frame::read(&mut &bytes[..])?;
        assert_eq!((reply.id, reply.kind, reply.body), (7, DATA, vec![0, 255]));
        bytes[16..20].copy_from_slice(&((codec::LIMIT as u32) + 1).to_be_bytes());
        assert!(Frame::read(&mut &bytes[..]).is_err());
        assert!(Frame::encode(0, FOLLOW, vec![]).is_err());
        Ok(())
    }
}
