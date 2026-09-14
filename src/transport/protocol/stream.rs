//! The incremental view of one run: the command's own bytes out, markers
//! stripped, source bytes counted.
//!
//! Only the command's own bytes reach the human sink; the wrapper's begin and
//! completion markers are never forwarded. Bytes before the begin marker are
//! login-profile noise and are dropped without being counted, because they are
//! not the command's output either. Everything is counted in *source bytes*,
//! independently of what the JSON later renders (EXEC-009).

use super::{Separator, begin_needle, done_needle, find_first, find_last};
use crate::domain::InvocationToken;
use crate::transport::process;
use std::io::{self, Write};

/// One streamed run, viewed incrementally.
pub struct CompletionStream<F: Write> {
    forward: Option<F>,
    capture: crate::transport::process::Capture,
    begin: Vec<u8>,
    done: Vec<u8>,
    pending: Vec<u8>,
    started: bool,
    completion: Option<i32>,
}

impl<F: Write> CompletionStream<F> {
    pub fn new(
        separator: Separator,
        nonce: &InvocationToken,
        forward: Option<F>,
        capture: Option<usize>,
    ) -> Self {
        Self {
            forward,
            // `None` means the caller is not keeping this stream at all.
            capture: process::Capture::new(
                capture.unwrap_or(0),
                if capture.is_some() {
                    process::Keep::Prefix
                } else {
                    process::Keep::Nothing
                },
            ),
            begin: begin_needle(separator, nonce),
            done: done_needle(separator, nonce),
            pending: Vec::new(),
            started: false,
            completion: None,
        }
    }

    /// The exit status carried by a completion marker bound to this invocation.
    /// `None` means no evidence, which is never a known success (EXEC-005).
    pub fn completion(&self) -> Option<i32> {
        self.completion
    }

    pub fn body(&mut self) -> Vec<u8> {
        self.capture.body()
    }

    pub fn total(&self) -> u64 {
        self.capture.total()
    }

    fn emit(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.capture.observe(bytes);
        if let Some(sink) = self.forward.as_mut() {
            sink.write_all(bytes)?;
            sink.flush()?;
        }
        Ok(())
    }

    /// Whether this run's own output began: without a begin marker nothing here
    /// is attributable to the command.
    pub fn started(&self) -> bool {
        self.started
    }
}

/// Length of the longest suffix of `data` that could still grow into `needle`.
fn partial_prefix(data: &[u8], needle: &[u8]) -> usize {
    let max = (needle.len() - 1).min(data.len());
    (1..=max)
        .rev()
        .find(|n| data[data.len() - n..] == needle[..*n])
        .unwrap_or(0)
}

impl<F: Write> crate::transport::process::Stream for CompletionStream<F> {
    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.pending.extend_from_slice(bytes);
        self.pump(false)
    }

    fn finish(&mut self) -> io::Result<()> {
        self.pump(true)?;
        if !self.started {
            // Without a begin marker this output is not attributable to the
            // command, so it is neither forwarded nor counted.
            self.pending.clear();
            return Ok(());
        }
        match self
            .done_offset()
            .zip(parse_after_done(&self.pending, &self.done))
        {
            Some((offset, code)) => {
                if offset > 0 {
                    let head = self.pending[..offset].to_vec();
                    self.emit(&head)?;
                }
                self.completion = Some(code);
            }
            // A completion marker whose status line never arrived is not
            // evidence; the bytes before it were output, so they stay visible.
            _ => {
                let tail = std::mem::take(&mut self.pending);
                self.emit(&tail)?;
            }
        }
        self.pending.clear();
        Ok(())
    }
}

impl<F: Write> CompletionStream<F> {
    fn done_offset(&self) -> Option<usize> {
        find_first(&self.pending, &self.done)
    }

    /// Forwards everything that is certainly the command's own output, holding
    /// back only as many bytes as could still grow into a marker.
    fn pump(&mut self, final_read: bool) -> io::Result<()> {
        loop {
            if !self.started {
                match find_first(&self.pending, &self.begin) {
                    Some(offset) => {
                        self.pending.drain(..offset + self.begin.len());
                        self.started = true;
                    }
                    None => {
                        let hold = self.begin.len() - 1;
                        let drop = self.pending.len().saturating_sub(hold);
                        self.pending.drain(..drop);
                        return Ok(());
                    }
                }
            }
            if let Some(offset) = self.done_offset() {
                if offset > 0 {
                    let head: Vec<u8> = self.pending.drain(..offset).collect();
                    self.emit(&head)?;
                }
                // The marker itself is protocol, not output: it stays in the
                // buffer until finish() reads the status out of it.
                return Ok(());
            }
            let hold = if final_read {
                0
            } else {
                partial_prefix(&self.pending, &self.done)
            };
            let flush = self.pending.len() - hold;
            if flush == 0 {
                return Ok(());
            }
            let head: Vec<u8> = self.pending.drain(..flush).collect();
            self.emit(&head)?;
        }
    }
}

fn parse_after_done(pending: &[u8], done: &[u8]) -> Option<i32> {
    let offset = find_last(pending, done)?;
    parse_status(&pending[offset + done.len()..])
}

fn parse_status(rest: &[u8]) -> Option<i32> {
    let end = rest.iter().position(|byte| *byte == b'\n')?;
    std::str::from_utf8(&rest[..end]).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use crate::transport::process::Stream;

    struct Sink(Vec<u8>);
    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[allow(clippy::type_complexity)]
    fn run_chunks(chunks: &[&[u8]], limit: usize) -> (Vec<u8>, u64, Option<i32>, Vec<u8>) {
        let nonce = InvocationToken::new("d".repeat(32)).unwrap_or_else(|e| panic!("{e}"));
        let mut sink = Sink(Vec::new());
        let mut stream =
            CompletionStream::new(Separator::Nul, &nonce, Some(&mut sink), Some(limit));
        for chunk in chunks {
            stream.feed(chunk).unwrap_or_else(|e| panic!("{e}"));
        }
        stream.finish().unwrap_or_else(|e| panic!("{e}"));
        let captured = stream.body();
        (captured, stream.total(), stream.completion(), sink.0)
    }

    #[test]
    fn markers_never_reach_the_human_stream() {
        let nonce = "d".repeat(32);
        let (captured, total, completion, forwarded) = run_chunks(
            &[
                b"profile noise\n",
                format!("\x00__RHOST_BEGIN_{nonce}__\nhello").as_bytes(),
                format!(" world\x00__RHOST_DONE_{nonce}__:3\n").as_bytes(),
            ],
            0,
        );
        assert_eq!(forwarded, b"hello world");
        assert_eq!(captured, b"hello world");
        assert_eq!(total, 11);
        assert_eq!(completion, Some(3));
    }

    #[test]
    fn a_marker_split_across_reads_is_still_observed() {
        let nonce = "d".repeat(32);
        let begin = format!("\x00__RHOST_BEGIN_{nonce}__\n").into_bytes();
        let done = format!("\x00__RHOST_DONE_{nonce}__:0\n").into_bytes();
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        chunks.push(b"noise".to_vec());
        chunks.extend(begin.chunks(3).map(<[u8]>::to_vec));
        chunks.push(b"payload".to_vec());
        chunks.extend(done.chunks(4).map(<[u8]>::to_vec));
        let refs: Vec<&[u8]> = chunks.iter().map(Vec::as_slice).collect();
        let (captured, total, completion, forwarded) = run_chunks(&refs, 0);
        assert_eq!(forwarded, b"payload");
        assert_eq!(captured, b"payload");
        assert_eq!(total, 7);
        assert_eq!(completion, Some(0));
    }

    #[test]
    fn an_unmatched_run_reports_no_completion() {
        let (captured, total, completion, forwarded) = run_chunks(&[b"partial output"], 0);
        assert!(captured.is_empty() && forwarded.is_empty() && completion.is_none());
        assert_eq!(total, 0);
    }

    #[test]
    fn a_truncated_capture_still_reports_source_bytes() {
        let nonce = "d".repeat(32);
        let mut head = format!("\0__RHOST_BEGIN_{nonce}__\n").into_bytes();
        head.extend_from_slice(&[b'x'; 5_000]);
        let tail = format!("\0__RHOST_DONE_{nonce}__:9\n").into_bytes();
        let (captured, total, completion, forwarded) = run_chunks(&[&head, &tail], 128);
        assert_eq!(captured.len(), 128);
        assert_eq!(forwarded.len(), 5_000);
        assert_eq!(total, 5_000);
        assert_eq!(completion, Some(9));
    }
}
