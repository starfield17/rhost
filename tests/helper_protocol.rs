//! Runs only an explicitly supplied helper; CI must not compile a substitute.
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};

fn call(input: &[u8]) -> Vec<u8> {
    let binary = std::env::var("RHOST_HELPER_BIN")
        .unwrap_or_else(|_| panic!("set RHOST_HELPER_BIN explicitly"));
    let mut child = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("spawn helper: {error}"));
    child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("stdin"))
        .write_all(input)
        .unwrap_or_else(|error| panic!("write: {error}"));
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("wait: {error}"));
    assert!(
        output.status.success(),
        "helper exited with {}",
        output.status
    );
    assert!(
        output.stderr.is_empty(),
        "helper leaked diagnostics outside its frame"
    );
    output.stdout
}

fn frame(kind: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = b"RH01".to_vec();
    bytes.extend(kind.to_be_bytes());
    bytes.extend((body.len() as u64).to_be_bytes());
    bytes.extend(body);
    bytes
}

fn payload(reply: &[u8], kind: u32) -> &[u8] {
    assert!(reply.len() >= 17);
    assert_eq!(&reply[..4], b"RH01");
    assert_eq!(&reply[4..8], &(kind | 0x80000000).to_be_bytes());
    let len = u64::from_be_bytes(reply[8..16].try_into().unwrap_or_else(|_| panic!("length")));
    assert_eq!(len, (reply.len() - 16) as u64);
    &reply[16..]
}

fn text<'a>(body: &mut &'a [u8]) -> &'a str {
    assert!(body.len() >= 4);
    let len =
        u32::from_be_bytes(body[..4].try_into().unwrap_or_else(|_| panic!("length"))) as usize;
    assert!(body.len() >= 4 + len);
    let value =
        std::str::from_utf8(&body[4..4 + len]).unwrap_or_else(|error| panic!("UTF-8: {error}"));
    *body = &body[4 + len..];
    value
}

#[test]
fn executable_probe_is_bounded_and_does_not_invent_capabilities() {
    let reply = call(&frame(1, &[]));
    let mut body = payload(&reply, 1);
    assert_eq!(body[0], 0);
    body = &body[1..];
    let digest = text(&mut body);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources: Vec<_> = std::fs::read_dir(root.join("helper"))
        .unwrap_or_else(|error| panic!("helper sources: {error}"))
        .map(|entry| {
            entry
                .unwrap_or_else(|error| panic!("source entry: {error}"))
                .path()
        })
        .filter(|path| matches!(path.extension().and_then(|s| s.to_str()), Some("c" | "h")))
        .collect();
    sources.sort();
    let mut hash = Sha256::new();
    for source in sources {
        let name = source
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_else(|| panic!("source name"));
        hash.update(format!("helper/{name}\0").as_bytes());
        hash.update(std::fs::read(source).unwrap_or_else(|error| panic!("read source: {error}")));
        hash.update([0]);
    }
    assert_eq!(
        digest,
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(digest.len(), 64);
    assert!(
        digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    assert!(matches!(
        text(&mut body),
        "linux" | "darwin" | "freebsd" | "openbsd" | "netbsd"
    ));
    assert!(!text(&mut body).is_empty());
    assert!(body.len() >= 4);
    assert_eq!(
        &body[..4],
        &7u32.to_be_bytes(),
        "verified Run, control and native files are advertised"
    );
    body = &body[4..];
    let boot = text(&mut body);
    assert!(matches!(boot.len(), 0 | 36));
    let image = text(&mut body);
    let bytes = std::fs::read(
        std::env::var_os("RHOST_HELPER_BIN").unwrap_or_else(|| panic!("set RHOST_HELPER_BIN")),
    )
    .unwrap_or_else(|e| panic!("image: {e}"));
    let expected_image = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    assert_eq!(image, expected_image);
    for expected in [8388608u32, 12582912, 8388608, 67108864, 1048576] {
        assert!(body.len() >= 4);
        assert_eq!(&body[..4], &expected.to_be_bytes());
        body = &body[4..];
    }
    assert!(body.is_empty());
}

#[test]
fn invalid_kinds_and_incomplete_bodies_are_not_unsupported_operations() {
    let mut wrong_magic = frame(1, &[]);
    wrong_magic[0] = b'X';
    let mut truncated = frame(1, b"abc");
    truncated.pop();
    for (input, kind, code) in [
        (wrong_magic, 1, "PROTOCOL_ERROR"),
        (frame(0, &[]), 0, "PROTOCOL_ERROR"),
        (frame(9, &[]), 9, "PROTOCOL_ERROR"),
        (truncated, 1, "PROTOCOL_ERROR"),
        (frame(2, &[]), 2, "PROTOCOL_ERROR"),
    ] {
        let reply = call(&input);
        let mut body = payload(&reply, kind);
        assert_eq!(body[0], 1);
        body = &body[1..];
        assert_eq!(text(&mut body), code);
    }
}

#[test]
fn malformed_frames_are_explicit_refusals() {
    let mut excessive = frame(1, &[]);
    excessive[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
    let mut trailing = frame(1, &[]);
    trailing.extend(frame(1, &[]));
    for input in [b"RH01".to_vec(), excessive, trailing, frame(1, b"extra")] {
        let reply = call(&input);
        let mut body = payload(&reply, if input.len() >= 8 { 1 } else { 0 });
        assert_eq!(body[0], 1);
        body = &body[1..];
        assert_eq!(text(&mut body), "PROTOCOL_ERROR");
        assert!(!text(&mut body).is_empty());
        assert_eq!(body, [0]);
    }
}

fn mux_frame(id: u64, kind: u32, body: &[u8]) -> Vec<u8> {
    let mut bytes = b"RH02".to_vec();
    bytes.extend(id.to_be_bytes());
    bytes.extend(kind.to_be_bytes());
    bytes.extend((body.len() as u32).to_be_bytes());
    bytes.extend(body);
    bytes
}

fn read_mux_frame(out: &mut impl Read) -> (u64, u32, Vec<u8>) {
    let mut header = [0u8; 20];
    out.read_exact(&mut header)
        .unwrap_or_else(|e| panic!("read RH02 header: {e}"));
    assert_eq!(&header[..4], b"RH02");
    let id = u64::from_be_bytes(header[4..12].try_into().unwrap_or_else(|_| panic!("id")));
    let kind = u32::from_be_bytes(header[12..16].try_into().unwrap_or_else(|_| panic!("kind")));
    let len =
        u32::from_be_bytes(header[16..20].try_into().unwrap_or_else(|_| panic!("len"))) as usize;
    let mut body = vec![0u8; len];
    if len > 0 {
        out.read_exact(&mut body)
            .unwrap_or_else(|e| panic!("read RH02 body: {e}"));
    }
    (id, kind, body)
}

#[test]
fn verified_mux_relays_tcp_and_reports_refused_connect() {
    let reply = call(&frame(1, &[]));
    let mut body = payload(&reply, 1);
    assert_eq!(body[0], 0);
    body = &body[1..];
    let digest = text(&mut body).to_string();
    let os = text(&mut body).to_string();
    let arch = text(&mut body).to_string();
    body = &body[4..];
    let _boot = text(&mut body);
    let image = text(&mut body).to_string();
    let abi = format!("{os}-{arch}");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind echo: {e}"));
    let open_port = listener
        .local_addr()
        .unwrap_or_else(|e| panic!("addr: {e}"))
        .port();
    let closed_listener =
        TcpListener::bind("127.0.0.1:0").unwrap_or_else(|e| panic!("bind closed: {e}"));
    let closed_port = closed_listener
        .local_addr()
        .unwrap_or_else(|e| panic!("closed addr: {e}"))
        .port();
    drop(closed_listener);

    let echo = std::thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let mut buf = [0u8; 64];
            while let Ok(n) = conn.read(&mut buf) {
                if n == 0 || conn.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    });

    let binary = std::env::var("RHOST_HELPER_BIN")
        .unwrap_or_else(|_| panic!("set RHOST_HELPER_BIN explicitly"));
    let mut child = Command::new(binary)
        .args(["--verified-mux", &digest, &image, &abi])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn mux: {e}"));
    let mut stdin = child.stdin.take().unwrap_or_else(|| panic!("stdin"));
    let mut stdout = child.stdout.take().unwrap_or_else(|| panic!("stdout"));

    let mut probe_header = [0u8; 16];
    stdout
        .read_exact(&mut probe_header)
        .unwrap_or_else(|e| panic!("handshake header: {e}"));
    assert_eq!(&probe_header[..4], b"RH01");
    let probe_len = u64::from_be_bytes(
        probe_header[8..16]
            .try_into()
            .unwrap_or_else(|_| panic!("probe len")),
    ) as usize;
    let mut probe_body = vec![0u8; probe_len];
    stdout
        .read_exact(&mut probe_body)
        .unwrap_or_else(|e| panic!("handshake body: {e}"));

    let mut open_body = Vec::new();
    let host = b"127.0.0.1";
    open_body.extend((host.len() as u32).to_be_bytes());
    open_body.extend(host);
    open_body.extend(u32::from(open_port).to_be_bytes());
    stdin
        .write_all(&mux_frame(1, 16, &open_body))
        .unwrap_or_else(|e| panic!("write open: {e}"));
    let (id, kind, body) = read_mux_frame(&mut stdout);
    assert_eq!(
        (id, kind, body.as_slice()),
        (1, 16 | 0x80000000, [0].as_slice())
    );

    stdin
        .write_all(&mux_frame(1, 17, b"ping"))
        .unwrap_or_else(|e| panic!("write data: {e}"));
    let (id, kind, body) = read_mux_frame(&mut stdout);
    assert_eq!(
        (id, kind, body.as_slice()),
        (1, 17 | 0x80000000, b"ping".as_slice())
    );

    stdin
        .write_all(&mux_frame(1, 18, &[1]))
        .unwrap_or_else(|e| panic!("write half-close: {e}"));
    let (id, kind, body) = read_mux_frame(&mut stdout);
    assert_eq!(
        (id, kind, body.as_slice()),
        (1, 18 | 0x80000000, [0].as_slice())
    );

    let mut closed_body = Vec::new();
    closed_body.extend((host.len() as u32).to_be_bytes());
    closed_body.extend(host);
    closed_body.extend(u32::from(closed_port).to_be_bytes());
    stdin
        .write_all(&mux_frame(2, 16, &closed_body))
        .unwrap_or_else(|e| panic!("write closed open: {e}"));
    let (id, kind, body) = read_mux_frame(&mut stdout);
    assert_eq!((id, kind), (2, 16 | 0x80000000));
    assert_eq!(body[0], 1);
    let mut rest = &body[1..];
    assert!(!text(&mut rest).is_empty());
    assert_eq!(rest, [1]);

    drop(stdin);
    let status = child.wait().unwrap_or_else(|e| panic!("wait mux: {e}"));
    assert!(status.success(), "mux exited with {status}");
    let _ = echo.join();
}
