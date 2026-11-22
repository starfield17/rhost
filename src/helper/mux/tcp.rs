//! Bidirectional outbound TCP streams multiplexed across an RH02 channel.
use super::{Channel, Frame, TCP_CLOSE, TCP_DATA, TCP_OPEN, unanswered};
use crate::{helper::codec, remote::Error};
use std::{
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError, SyncSender},
    },
    time::Duration,
};

const TCP_MAX_STREAMS: usize = 64;
const TCP_CHUNK: usize = 16384;

pub(crate) enum TcpOpenError {
    Refused,
    Other,
    Channel,
}

pub(crate) enum TcpEvent {
    Data(Vec<u8>),
    HalfClose,
    Closed,
}

#[derive(Clone)]
pub(crate) struct TcpWriter {
    id: u64,
    writer: SyncSender<Vec<u8>>,
    routing: Weak<super::Routing>,
    closed: Arc<AtomicBool>,
}

pub(crate) struct TcpStream {
    writer: TcpWriter,
    rx: mpsc::Receiver<Frame>,
}

impl Channel {
    pub(crate) fn is_alive(&self) -> bool {
        self.routing.alive.load(Ordering::Acquire)
    }

    pub(crate) fn open_tcp(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<TcpStream, TcpOpenError> {
        if !self.is_alive() {
            return Err(TcpOpenError::Channel);
        }
        let mut body = crate::helper::Writer::default();
        body.text(host).map_err(|_| TcpOpenError::Channel)?;
        body.int(u32::from(port));
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let wire = Frame::encode(id, TCP_OPEN, body.0).map_err(|_| TcpOpenError::Channel)?;
        let (send, rx) = mpsc::channel();
        {
            let mut streams = self
                .routing
                .streams
                .lock()
                .map_err(|_| TcpOpenError::Channel)?;
            if !self.is_alive() {
                return Err(TcpOpenError::Channel);
            }
            if streams.len() >= TCP_MAX_STREAMS {
                return Err(TcpOpenError::Other);
            }
            streams.insert(id, send);
        }
        // Blocking send waits for the writer thread rather than dropping control frames.
        if self.writer.send(wire).is_err() {
            if let Ok(mut streams) = self.routing.streams.lock() {
                streams.remove(&id);
            }
            return Err(TcpOpenError::Channel);
        }
        match rx.recv_timeout(timeout) {
            Ok(frame) if frame.kind == TCP_OPEN => {
                parse_open_reply(&frame.body).map(|()| TcpStream {
                    writer: TcpWriter {
                        id,
                        writer: self.writer.clone(),
                        routing: Arc::downgrade(&self.routing),
                        closed: Arc::new(AtomicBool::new(false)),
                    },
                    rx,
                })
            }
            Ok(_) => {
                if let Ok(mut streams) = self.routing.streams.lock() {
                    streams.remove(&id);
                }
                Err(TcpOpenError::Channel)
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = close_stream(id, &self.writer, &self.routing);
                Err(TcpOpenError::Other)
            }
            Err(RecvTimeoutError::Disconnected) => Err(TcpOpenError::Channel),
        }
    }
}

fn parse_open_reply(body: &[u8]) -> Result<(), TcpOpenError> {
    let mut reader = codec::Reader::new(body);
    match reader.byte().map_err(|_| TcpOpenError::Channel)? {
        0 => {
            reader.done().map_err(|_| TcpOpenError::Channel)?;
            Ok(())
        }
        1 => {
            let _message = reader.text().map_err(|_| TcpOpenError::Channel)?;
            let refused = reader.byte().map_err(|_| TcpOpenError::Channel)?;
            reader.done().map_err(|_| TcpOpenError::Channel)?;
            if refused == 1 {
                Err(TcpOpenError::Refused)
            } else {
                Err(TcpOpenError::Other)
            }
        }
        _ => Err(TcpOpenError::Channel),
    }
}

fn close_stream(
    id: u64,
    writer: &SyncSender<Vec<u8>>,
    routing: &super::Routing,
) -> Result<(), Error> {
    if let Ok(mut streams) = routing.streams.lock() {
        streams.remove(&id);
    }
    let frame = Frame::encode(id, TCP_CLOSE, vec![0])?;
    let _ = writer.send(frame);
    Ok(())
}

impl TcpWriter {
    pub(crate) fn send_data(&self, bytes: &[u8]) -> Result<(), Error> {
        for chunk in bytes.chunks(TCP_CHUNK) {
            let frame = Frame::encode(self.id, TCP_DATA, chunk.to_vec())?;
            self.writer.send(frame).map_err(|_| unanswered())?;
        }
        Ok(())
    }

    pub(crate) fn half_close(&self) -> Result<(), Error> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let frame = Frame::encode(self.id, TCP_CLOSE, vec![1])?;
        self.writer.send(frame).map_err(|_| unanswered())
    }

    pub(crate) fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(routing) = self.routing.upgrade() {
            let _ = close_stream(self.id, &self.writer, &routing);
        } else if let Ok(frame) = Frame::encode(self.id, TCP_CLOSE, vec![0]) {
            let _ = self.writer.send(frame);
        }
    }
}

impl TcpStream {
    pub(crate) fn split(self) -> (TcpWriter, TcpReader) {
        let writer = self.writer;
        (
            writer.clone(),
            TcpReader {
                rx: self.rx,
                writer,
            },
        )
    }
}

pub(crate) struct TcpReader {
    rx: mpsc::Receiver<Frame>,
    writer: TcpWriter,
}

impl TcpReader {
    pub(crate) fn recv(&self) -> TcpEvent {
        match self.rx.recv() {
            Ok(frame) if frame.kind == TCP_DATA => TcpEvent::Data(frame.body),
            Ok(frame) if frame.kind == TCP_CLOSE => {
                if frame.body.first().copied() == Some(1) {
                    TcpEvent::HalfClose
                } else {
                    TcpEvent::Closed
                }
            }
            _ => TcpEvent::Closed,
        }
    }
}

impl Drop for TcpReader {
    fn drop(&mut self) {
        self.writer.close();
    }
}
