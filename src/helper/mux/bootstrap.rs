//! Verify the helper before any RH02 request can reach SSH stdin.
use super::{Channel, INTERVAL, before, cancelled, unanswered};
use crate::helper::{
    Helper, codec,
    verified::{Mode, transport_refusal},
};
use crate::{domain::Recovery, remote::Error};
use std::io::{self, Read};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};
const OPEN_BUDGET: Duration = Duration::from_secs(15);

pub(super) fn handshake(input: &mut impl Read, marker: &str) -> Result<Vec<u8>, Error> {
    let read = |_| before(unanswered());
    let mut prefix = Vec::new();
    let mut byte = [0];
    while !prefix.ends_with(marker.as_bytes()) {
        if prefix.len() == 65536 {
            return Err(before(codec::protocol("missing RH02 bootstrap marker")));
        }
        input.read_exact(&mut byte).map_err(read)?;
        prefix.push(byte[0]);
    }
    let mut header = [0; 16];
    input.read_exact(&mut header).map_err(read)?;
    if &header[..4] != b"RH01" {
        let mut receipt = header.to_vec();
        while !receipt.contains(&b'\n') && receipt.len() < 80 {
            input.read_exact(&mut byte).map_err(read)?;
            receipt.push(byte[0]);
        }
        let (code, message) = match receipt.as_slice() {
            b"RHOST_FAST_MISSING\n" => ("HELPER_PREPARATION_FAILED", "matching helper is absent"),
            b"RHOST_FAST_DEPENDENCY\n" => (
                "REMOTE_DEPENDENCY_MISSING",
                "native bootstrap dependency is unavailable",
            ),
            b"RHOST_FAST_UNSUPPORTED\n" => (
                "UNSUPPORTED_REMOTE_OS",
                "native bootstrap target is unsupported",
            ),
            b"RHOST_FAST_FIXED_REFUSED\n" => {
                ("HELPER_INCOMPATIBLE", "fixed helper failed verification")
            }
            _ => return Err(before(codec::protocol("invalid RH02 bootstrap response"))),
        };
        return Err(Error::new(code, message).with_recovery(Recovery::preparation()));
    }
    let size = codec::Reader::new(&header[8..]).long()?;
    if !(1..=1024).contains(&size) {
        return Err(before(codec::protocol("invalid RH02 probe length")));
    }
    let mut wire = header.to_vec();
    wire.resize(16 + size as usize, 0);
    input.read_exact(&mut wire[16..]).map_err(read)?;
    codec::answer(&wire, 1).map_err(before)
}

impl Helper<'_> {
    fn connect_mux(&self) -> Result<Channel, Error> {
        if (self.cancelled)() {
            return Err(before(cancelled()));
        }
        let nonce =
            crate::random::hex(16).map_err(|e| before(Error::new("INTERNAL", e.to_string())))?;
        let marker = format!("RHOST_MUX_{nonce}\n");
        let command = self
            .verified_command(&marker, Mode::Mux, false)
            .map_err(|e| e.with_recovery(Recovery::preparation()))?;
        let child = self
            .client
            .spawn_channel(self.host, &command)
            .map_err(|e| before(launch_error(e)))?;
        let (channel, bootstrap) = Channel::new(child, marker)?;
        let start = Instant::now();
        loop {
            if (self.cancelled)() {
                return Err(before(cancelled()));
            }
            match bootstrap.ready.recv_timeout(INTERVAL) {
                Ok(Err(error)) if error.code == "REMOTE_EXECUTION_UNKNOWN" => {
                    if let Ok(stderr) = bootstrap.stderr.recv_timeout(Duration::from_millis(100)) {
                        let exit = channel.child.lock().ok().and_then(|mut child| {
                            child
                                .try_wait()
                                .ok()
                                .flatten()
                                .and_then(|status| status.code())
                        });
                        if let Some(refusal) = transport_refusal(&stderr, exit) {
                            return Err(refusal);
                        }
                    }
                    return Err(error);
                }
                Ok(probe) => {
                    let pin = if Self::explicit_selection() {
                        self.expected_image()?
                    } else {
                        None
                    };
                    let _ = self.decode_probe(&probe?, pin.as_deref()).map_err(before)?;
                    return Ok(channel);
                }
                Err(RecvTimeoutError::Timeout) if start.elapsed() < OPEN_BUDGET => {}
                Err(_) => return Err(before(unanswered())),
            }
        }
    }

    pub(super) fn open_mux(&self, prepare: bool) -> Result<Channel, Error> {
        match self.connect_mux() {
            Err(error) if prepare && !self.fixed() && error.code == "HELPER_PREPARATION_FAILED" => {
                self.ensure()
                    .map_err(|e| e.with_recovery(Recovery::preparation()))?;
                self.connect_mux()
            }
            result => result,
        }
    }
}
fn launch_error(error: io::Error) -> Error {
    Error::new("CONFIG_INVALID", error.to_string())
}
