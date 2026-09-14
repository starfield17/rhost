//! The bounded copy of one stream, and the sinks that write into one.
//!
//! A capture never unbounds: whatever the mode, the bytes kept in memory are a
//! prefix plus, where the completion marker needs it, a fixed-size suffix. Every
//! byte is counted, so a truncated body still reports the true size.

use std::io::{self, Write};

/// How much of a stream rhost keeps in memory: a prefix plus a fixed
/// protocol-sized suffix, while counting every byte.
///
/// Keeping only the first N bytes would break the completion protocol, because
/// the marker carrying the real exit status is the last thing on stdout. The
/// suffix only has to hold one marker, which is a property of rhost's own
/// protocol rather than of the command — that is what makes the bound safe
/// rather than merely small.
pub const TAIL_KEEP: usize = 256;

/// How much of a stream is retained in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// A buffered run has to keep the completion marker at the end of stdout
    /// readable, so a capped capture keeps a prefix *and* a protocol-sized
    /// suffix.
    PrefixAndMarkerTail,
    /// A streamed run filters the markers out before capturing, so nothing in
    /// the tail is the command's own output: a prefix is enough.
    Prefix,
    /// Count only. A human watching a terminal needs the bytes forwarded, not
    /// held: `--max-output-bytes` bounds what rhost *carries back*, and a human
    /// run carries nothing back.
    Nothing,
}

/// Keeps a bounded copy of a byte stream while counting every byte seen.
#[derive(Debug)]
pub struct Capture {
    limit: usize,
    keep: Keep,
    head: Vec<u8>,
    tail: Vec<u8>,
    total: u64,
}

impl Capture {
    /// A `limit` of 0 keeps everything the mode retains.
    pub fn new(limit: usize, keep: Keep) -> Self {
        Self {
            limit,
            keep,
            head: Vec::new(),
            tail: Vec::new(),
            total: 0,
        }
    }

    pub fn observe(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        if self.keep == Keep::Nothing {
            return;
        }
        if self.limit == 0 {
            self.head.extend_from_slice(bytes);
            return;
        }
        let mut source = bytes;
        let room = self.limit.saturating_sub(self.head.len());
        if room > 0 {
            let take = room.min(source.len());
            self.head.extend_from_slice(&source[..take]);
            source = &source[take..];
        }
        if source.is_empty() || self.keep != Keep::PrefixAndMarkerTail {
            return;
        }
        if source.len() >= TAIL_KEEP {
            self.tail.clear();
            self.tail
                .extend_from_slice(&source[source.len() - TAIL_KEEP..]);
        } else {
            let overflow = (self.tail.len() + source.len()).saturating_sub(TAIL_KEEP);
            if overflow > 0 {
                self.tail.drain(..overflow);
            }
            self.tail.extend_from_slice(source);
        }
    }

    /// The retained prefix, followed by the retained suffix where one is kept.
    /// The two are contiguous whenever the total stayed inside limit +
    /// TAIL_KEEP, which is the interesting case: output that only just exceeded
    /// the cap arrives whole.
    pub fn body(&self) -> Vec<u8> {
        if self.keep != Keep::PrefixAndMarkerTail {
            return self.head.clone();
        }
        let mut out = Vec::with_capacity(self.head.len() + self.tail.len());
        out.extend_from_slice(&self.head);
        out.extend_from_slice(&self.tail);
        out
    }

    pub fn total(&self) -> u64 {
        self.total
    }
}

/// What one stream does with the bytes it receives. Implemented twice: plain
/// forwarding for a human, and marker-filtered capture for an agent.
pub trait Stream {
    fn feed(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Called once, after the child's output end reached EOF.
    fn finish(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Forwards bytes to a caller-owned sink while keeping a bounded copy.
///
/// A sink that stops accepting bytes ends the run: output that cannot be
/// delivered is a delivery failure, never a rerun (EXEC-010).
pub struct Tap {
    forward: Option<Box<dyn Write>>,
    capture: Capture,
}

impl Tap {
    pub fn new(forward: Option<Box<dyn Write>>, limit: usize, keep: Keep) -> Self {
        Self {
            forward,
            capture: Capture::new(limit, keep),
        }
    }

    pub fn body(&self) -> Vec<u8> {
        self.capture.body()
    }

    pub fn total(&self) -> u64 {
        self.capture.total()
    }
}

impl Stream for Tap {
    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.capture.observe(bytes);
        if let Some(sink) = self.forward.as_mut() {
            sink.write_all(bytes)?;
            sink.flush()?;
        }
        Ok(())
    }
}

/// Keeps a bounded copy of a stream for callers that read the whole result once
/// the child is gone.
pub struct Recorder(Tap);

impl Recorder {
    pub fn new(limit: usize) -> Self {
        Self(Tap::new(None, limit, Keep::PrefixAndMarkerTail))
    }
    pub fn body(&self) -> Vec<u8> {
        self.0.body()
    }
    pub fn total(&self) -> u64 {
        self.0.total()
    }
}

impl Stream for Recorder {
    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.feed(bytes)
    }
}
