//! Runs only an explicitly supplied helper; CI must not compile a substitute.
use std::io::Write;
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
    assert_eq!(
        body,
        0u32.to_be_bytes(),
        "scaffolding must advertise no unimplemented operations"
    );
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
