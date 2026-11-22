//! Exact-byte C helper acceptance through its verified duplex entry.
use super::{Reader, Sandbox, key, poll, start, text};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

type Reply = (u64, u32, Vec<u8>);
struct Mux {
    child: Child,
    input: Option<ChildStdin>,
    replies: Receiver<std::io::Result<Reply>>,
}

impl Mux {
    fn open(
        state: &Sandbox,
        cgroup: Option<&std::path::Path>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let binary = std::env::var_os("RHOST_HELPER_BIN").ok_or("set RHOST_HELPER_BIN")?;
        let hash: String = Sha256::digest(std::fs::read(&binary)?)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let arch = match std::env::consts::ARCH {
            "arm" => "armv7-eabihf",
            arch => arch,
        };
        let os = match std::env::consts::OS {
            "macos" => "darwin",
            os => os,
        };
        let abi = format!("{os}-{arch}");
        let mut command = Command::new(binary);
        command
            .args([
                "--verified-mux",
                &rhost::helper::source_digest(),
                &hash,
                &abi,
            ])
            .env("RHOST_REMOTE_STATE", &state.0)
            .env_remove("RHOST_CGROUP_ROOT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(path) = cgroup {
            command.env("RHOST_CGROUP_ROOT", path);
        }
        let mut child = command.spawn()?;
        let mut output = child.stdout.take().ok_or("helper output missing")?;
        let input = child.stdin.take().ok_or("helper input missing")?;
        let mut probe = [0; 16];
        output.read_exact(&mut probe)?;
        assert_eq!(&probe[..8], b"RH01\x80\0\0\x01");
        let size = Reader(&probe[8..]).long() as usize;
        assert!((1..=1024).contains(&size));
        let mut body = vec![0; size];
        output.read_exact(&mut body)?;
        assert_eq!(body[0], 0);
        let (send, replies) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            loop {
                let reply = read_reply(&mut output);
                let end = reply.is_err();
                if send.send(reply).is_err() || end {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input: Some(input),
            replies,
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), std::io::Error> {
        self.input
            .as_mut()
            .ok_or(std::io::ErrorKind::BrokenPipe)?
            .write_all(bytes)
    }

    fn get(&self) -> Result<Reply, Box<dyn std::error::Error>> {
        Ok(self.replies.recv_timeout(Duration::from_secs(5))??)
    }

    fn disconnect(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        drop(self.input.take());
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait()? {
                assert!(status.success(), "{status}");
                return Ok(());
            }
            if Instant::now() >= until {
                return Err("RH02 helper did not exit on stdin EOF".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Mux {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_reply(input: &mut impl Read) -> std::io::Result<Reply> {
    let mut header = [0; 20];
    input.read_exact(&mut header)?;
    assert_eq!(&header[..4], b"RH02");
    let mut fields = Reader(&header[4..]);
    let id = fields.long();
    let kind = fields.int();
    assert_ne!(kind & 0x80000000, 0);
    let size = fields.int() as usize;
    assert!(size <= 8 * 1024 * 1024);
    let mut body = vec![0; size];
    input.read_exact(&mut body)?;
    Ok((id, kind & !0x80000000, body))
}

fn frame(id: u64, kind: u32, body: &[u8]) -> Vec<u8> {
    let mut wire = b"RH02".to_vec();
    wire.extend(id.to_be_bytes());
    wire.extend(kind.to_be_bytes());
    wire.extend((body.len() as u32).to_be_bytes());
    wire.extend(body);
    wire
}

fn follow(g: &str, k: &str, out: u64, err: u64, wait: u32, cap: u32) -> Vec<u8> {
    let mut body = Vec::new();
    text(&mut body, g);
    text(&mut body, k);
    body.extend(out.to_be_bytes());
    body.extend(err.to_be_bytes());
    body.extend(wait.to_be_bytes());
    body.extend(cap.to_be_bytes());
    body
}

#[test]
fn rh02_pipelines_independent_replies_and_disconnect_preserves_detached_runs()
-> Result<(), Box<dyn std::error::Error>> {
    let state = Sandbox::new();
    let g = state.prepare();
    let (k, expiry) = key();
    let mut channel = Mux::open(&state, None)?;
    let release = state.0.join("release");
    let script = format!(
        "while [ ! -e {} ]; do sleep 0.05; done; printf done",
        rhost::shell::quote(&release.to_string_lossy())
    );
    let request = frame(101, 2, &start(&g, &k, expiry, &script, 0, 15000));
    channel.write(&request)?;
    let (id, kind, body) = channel.get()?;
    assert_eq!((id, kind, body[0]), (101, 2, 0));
    let mut pipelined = frame(102, 11, &follow(&g, &k, 0, 0, 30000, 0));
    pipelined.extend(frame(103, 8, &[]));
    channel.write(&pipelined)?;
    let (id, kind, body) = channel.get()?;
    assert_eq!((id, kind, body[0]), (103, 8, 0));
    channel.disconnect()?;
    std::fs::write(release, [])?;
    let mut watched = Vec::new();
    text(&mut watched, &g);
    text(&mut watched, &k);
    watched.extend(5000u32.to_be_bytes());
    let bytes = state.call(9, &watched);
    let mut observed = Reader(&bytes);
    assert_eq!(observed.byte(), 0);
    assert_eq!(observed.text(), g);
    assert_eq!(observed.text(), k);
    assert_eq!(observed.byte(), 4);
    let bytes = state.call(3, &poll(&g, &k, 0, 0));
    assert_eq!(super::completed(&bytes, &g, &k, 0).0, b"done");
    Ok(())
}

#[test]
fn rh02_partial_mutation_is_never_dispatched() -> Result<(), Box<dyn std::error::Error>> {
    let state = Sandbox::new();
    let g = state.prepare();
    let (k, expiry) = key();
    let request = frame(99, 2, &start(&g, &k, expiry, "exit 42", 0, 0));
    let mut channel = Mux::open(&state, None)?;
    channel.write(&request[..request.len() - 1])?;
    channel.disconnect()?;
    let reply = state.call(3, &poll(&g, &k, 0, 0));
    let mut reader = Reader(&reply);
    assert_eq!(reader.byte(), 1);
    assert_eq!(reader.text(), "RUN_NOT_FOUND");
    Ok(())
}

#[test]
fn follow_emits_leader_completion_and_resumes_at_each_raw_offset()
-> Result<(), Box<dyn std::error::Error>> {
    let state = Sandbox::new();
    let g = state.prepare();
    let (k, expiry) = key();
    let release = state.0.join("release");
    let script = format!(
        "printf ab; printf X >&2; ( for i in 1 2 3 4 5 6 7 8 9 10; do [ ! -e {} ] || break; sleep 0.2; done; printf cd; printf Y >&2 ) & exit 17",
        rhost::shell::quote(&release.to_string_lossy())
    );
    let started = state.call(2, &start(&g, &k, expiry, &script, 1000, 0));
    assert_eq!(
        super::completed(&started, &g, &k, 17),
        (b"ab".to_vec(), b"X".to_vec())
    );
    let mut channel = Mux::open(&state, None)?;
    channel.write(&frame(1, 11, &follow(&g, &k, 0, 0, 5000, 2)))?;
    let mut result = [Vec::new(), Vec::new()];
    let mut completion = false;
    loop {
        let (id, kind, body) = channel.get()?;
        assert_eq!(id, 1);
        let mut r = Reader(&body);
        match kind {
            11 => {
                assert_eq!(r.byte(), 0);
                break;
            }
            12 => {
                let stream = r.byte() as usize - 1;
                assert_eq!(r.long(), result[stream].len() as u64);
                result[stream].extend(r.blob());
                assert!(r.0.is_empty());
            }
            13 => {
                assert_eq!(r.byte(), 0);
                assert_eq!(r.text(), g);
                assert_eq!(r.text(), k);
                assert_eq!((r.byte(), r.byte(), r.int()), (4, 1, 17));
                assert_eq!((r.byte(), r.byte()), (0, 0));
                let _observed = r.long();
                let _retained = r.long();
                assert_eq!(r.byte(), 1, "leader completion may precede pipe EOF");
                completion = true;
            }
            _ => return Err("unexpected FOLLOW frame".into()),
        }
    }
    assert!(completion);
    assert_eq!(&result[0], b"ab");
    assert_eq!(&result[1], b"X");
    channel.disconnect()?;
    std::fs::write(release, [])?;
    let mut channel = Mux::open(&state, None)?;
    channel.write(&frame(1, 11, &follow(&g, &k, 2, 1, 5000, 16)))?;
    loop {
        let (_, kind, body) = channel.get()?;
        let mut r = Reader(&body);
        if kind == 11 {
            assert_eq!(r.byte(), 0);
            break;
        }
        if kind == 12 {
            let stream = r.byte() as usize - 1;
            assert_eq!(r.long(), result[stream].len() as u64);
            result[stream].extend(r.blob());
        }
    }
    assert_eq!(&result[0], b"abcd");
    assert_eq!(&result[1], b"XY");
    Ok(())
}

#[test]
fn cgroup_unavailable_refuses_before_exec() -> Result<(), Box<dyn std::error::Error>> {
    let state = Sandbox::new();
    let g = state.prepare();
    let (id, expiry) = key();
    let mut body = start(&g, &id, expiry, "exit 99", 0, 0);
    body.push(1);
    let mut channel = Mux::open(&state, Some(&state.0))?;
    channel.write(&frame(1, 2, &body))?;
    let (request, kind, reply) = channel.get()?;
    assert_eq!((request, kind), (1, 2));
    let mut reply = Reader(&reply);
    assert_eq!(reply.byte(), 1);
    assert_eq!(reply.text(), "CONTAINMENT_UNAVAILABLE");
    let absent = state.call(3, &poll(&g, &id, 0, 0));
    let mut absent = Reader(&absent);
    assert_eq!(absent.byte(), 1);
    assert_eq!(absent.text(), "RUN_NOT_FOUND");
    Ok(())
}

#[test]
fn a_missing_worker_reply_is_scoped_to_its_request() -> Result<(), Box<dyn std::error::Error>> {
    let state = Sandbox::new();
    state.prepare();
    let mut channel = Mux::open(&state, None)?;
    let mut file = 1u32.to_be_bytes().to_vec();
    file.extend(br#"{"op":"invalid"}"#);
    let mut pipeline = frame(1, 15, &file);
    pipeline.extend(frame(2, 8, &[]));
    channel.write(&pipeline)?;
    let mut replies = [channel.get()?, channel.get()?];
    replies.sort_by_key(|reply| reply.0);
    assert_eq!((replies[0].0, replies[0].1), (1, 15));
    let mut refusal = Reader(&replies[0].2);
    assert_eq!(refusal.byte(), 1);
    assert_eq!(refusal.text(), "REMOTE_EXECUTION_UNKNOWN");
    assert_eq!((replies[1].0, replies[1].1, replies[1].2[0]), (2, 8, 0));
    channel.write(&frame(3, 8, &[]))?;
    assert_eq!(channel.get()?.0, 3);
    channel.disconnect()?;
    Ok(())
}
