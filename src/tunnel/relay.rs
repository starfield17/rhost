//! SOCKS5 handshake and bidirectional stream relay for helper tunnels.
use crate::{
    helper::mux::{Channel, TcpEvent, TcpOpenError, TcpStream as MuxTcpStream},
    tunnel::record::{DestinationEvidence, Kind},
};
use std::{
    io::{ErrorKind, Read, Write},
    net::{Ipv4Addr, Ipv6Addr, Shutdown, TcpStream},
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn store_evidence(cell: &AtomicU8, evidence: DestinationEvidence) {
    let code = match evidence {
        DestinationEvidence::Unknown => 0,
        DestinationEvidence::Reachable => 1,
        DestinationEvidence::Refused => 2,
    };
    cell.store(code, Ordering::Release);
}

pub(super) fn load_evidence(cell: &AtomicU8) -> DestinationEvidence {
    match cell.load(Ordering::Acquire) {
        1 => DestinationEvidence::Reachable,
        2 => DestinationEvidence::Refused,
        _ => DestinationEvidence::Unknown,
    }
}

pub(super) fn handle_client(
    mut local: TcpStream,
    channel: &Channel,
    kind: Kind,
    destination: Option<(String, u16)>,
    evidence: &AtomicU8,
) {
    let _ = local.set_nodelay(true);
    let remote = match kind {
        Kind::Local => {
            let Some((host, port)) = destination else {
                return;
            };
            match dial_remote(channel, &host, port, evidence) {
                Ok(remote) => remote,
                Err(_) => return,
            }
        }
        Kind::Socks => match socks5_connect(&mut local, channel, evidence) {
            Some(remote) => remote,
            None => return,
        },
        Kind::Reverse => return,
    };
    relay(local, remote);
}

fn dial_remote(
    channel: &Channel,
    host: &str,
    port: u16,
    evidence: &AtomicU8,
) -> Result<MuxTcpStream, u8> {
    let trimmed = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    match channel.open_tcp(trimmed, port, CONNECT_TIMEOUT) {
        Ok(remote) => {
            store_evidence(evidence, DestinationEvidence::Reachable);
            Ok(remote)
        }
        Err(TcpOpenError::Refused) => {
            store_evidence(evidence, DestinationEvidence::Refused);
            Err(0x05)
        }
        Err(_) => Err(0x01),
    }
}

fn socks5_connect(
    local: &mut TcpStream,
    channel: &Channel,
    evidence: &AtomicU8,
) -> Option<MuxTcpStream> {
    let _ = local.set_read_timeout(Some(HANDSHAKE_TIMEOUT));
    let _ = local.set_write_timeout(Some(HANDSHAKE_TIMEOUT));
    let mut head = [0u8; 2];
    local.read_exact(&mut head).ok()?;
    if head[0] != 5 || head[1] == 0 {
        return None;
    }
    let mut methods = vec![0u8; usize::from(head[1])];
    local.read_exact(&mut methods).ok()?;
    if !methods.contains(&0x00) {
        let _ = local.write_all(&[0x05, 0xff]);
        return None;
    }
    local.write_all(&[0x05, 0x00]).ok()?;

    let mut req = [0u8; 4];
    local.read_exact(&mut req).ok()?;
    if req[0] != 5 || req[1] != 1 {
        let _ = local.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        return None;
    }
    let host = match req[3] {
        1 => {
            let mut ip = [0u8; 4];
            local.read_exact(&mut ip).ok()?;
            Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let mut len = [0u8; 1];
            local.read_exact(&mut len).ok()?;
            if len[0] == 0 {
                let _ = local.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
                return None;
            }
            let mut domain = vec![0u8; usize::from(len[0])];
            local.read_exact(&mut domain).ok()?;
            String::from_utf8(domain).ok()?
        }
        4 => {
            let mut ip = [0u8; 16];
            local.read_exact(&mut ip).ok()?;
            Ipv6Addr::from(ip).to_string()
        }
        _ => {
            let _ = local.write_all(&[0x05, 0x08, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
            return None;
        }
    };
    let mut port_bytes = [0u8; 2];
    local.read_exact(&mut port_bytes).ok()?;
    let port = u16::from_be_bytes(port_bytes);
    if port == 0 {
        let _ = local.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        return None;
    }
    match dial_remote(channel, &host, port, evidence) {
        Ok(remote) => {
            local
                .write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                .ok()?;
            let _ = local.flush();
            let _ = local.set_read_timeout(None);
            let _ = local.set_write_timeout(None);
            Some(remote)
        }
        Err(rep) => {
            let _ = local.write_all(&[0x05, rep, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
            None
        }
    }
}

fn relay(mut local_rx: TcpStream, remote: MuxTcpStream) {
    let Ok(mut local_tx) = local_rx.try_clone() else {
        return;
    };
    let (remote_tx, remote_rx) = remote.split();
    let up = std::thread::spawn(move || {
        let mut buf = vec![0u8; 16384];
        loop {
            match local_rx.read(&mut buf) {
                Ok(0) => {
                    let _ = remote_tx.half_close();
                    break;
                }
                Ok(n) => {
                    if remote_tx.send_data(&buf[..n]).is_err() {
                        remote_tx.close();
                        break;
                    }
                }
                Err(ref error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    remote_tx.close();
                    break;
                }
            }
        }
    });
    loop {
        match remote_rx.recv() {
            TcpEvent::Data(bytes) => {
                if local_tx.write_all(&bytes).is_err() {
                    break;
                }
            }
            TcpEvent::HalfClose => {
                let _ = local_tx.shutdown(Shutdown::Write);
            }
            TcpEvent::Closed => {
                let _ = local_tx.shutdown(Shutdown::Both);
                break;
            }
        }
    }
    drop(remote_rx);
    let _ = up.join();
}
