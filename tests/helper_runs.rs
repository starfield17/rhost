//! Independent process-level acceptance of remote-owned Run primitives.
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let parent = std::fs::canonicalize("target").unwrap_or_else(|e| panic!("target: {e}"));
        Self(parent.join(format!(
            "run-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )))
    }
    fn call(&self, kind: u32, body: &[u8]) -> Vec<u8> {
        let helper =
            std::env::var("RHOST_HELPER_BIN").unwrap_or_else(|_| panic!("set RHOST_HELPER_BIN"));
        let mut child = Command::new(helper)
            .env("RHOST_REMOTE_STATE", &self.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("helper: {e}"));
        let mut frame = b"RH01".to_vec();
        frame.extend(kind.to_be_bytes());
        frame.extend((body.len() as u64).to_be_bytes());
        frame.extend(body);
        child
            .stdin
            .take()
            .unwrap_or_else(|| panic!("stdin"))
            .write_all(&frame)
            .unwrap_or_else(|e| panic!("input: {e}"));
        let result = child
            .wait_with_output()
            .unwrap_or_else(|e| panic!("wait: {e}"));
        assert!(result.status.success(), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert!(result.stdout.len() >= 17, "missing response");
        assert_eq!(&result.stdout[..4], b"RH01");
        assert_eq!(&result.stdout[4..8], &(kind | 0x80000000).to_be_bytes());
        assert_eq!(
            u64::from_be_bytes(
                result.stdout[8..16]
                    .try_into()
                    .unwrap_or_else(|_| panic!("length"))
            ),
            (result.stdout.len() - 16) as u64
        );
        result.stdout[16..].to_vec()
    }
    fn prepare(&self) -> String {
        let reply = self.call(7, &[]);
        let mut r = Reader(&reply);
        assert_eq!(r.byte(), 0, "{reply:?}");
        let generation = r.text();
        assert_eq!(generation.len(), 32);
        assert!(r.long() > 0);
        assert!(r.0.is_empty());
        generation
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn text(b: &mut Vec<u8>, s: &str) {
    b.extend((s.len() as u32).to_be_bytes());
    b.extend(s.as_bytes());
}
fn key() -> (String, u64) {
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|e| panic!("time: {e}"))
        .as_secs()
        + 3600;
    (format!("{expiry}-{}", "b".repeat(64)), expiry)
}
fn start(
    generation: &str,
    key: &str,
    expiry: u64,
    script: &str,
    wait: u32,
    deadline: u64,
) -> Vec<u8> {
    let mut b = Vec::new();
    text(&mut b, generation);
    text(&mut b, key);
    b.extend(expiry.to_be_bytes());
    text(&mut b, "/tmp");
    b.push(1);
    b.extend(3u32.to_be_bytes());
    for a in ["/bin/sh", "-c", script] {
        text(&mut b, a);
    }
    b.push(0);
    b.extend(0u32.to_be_bytes());
    b.extend(0u32.to_be_bytes());
    b.extend(wait.to_be_bytes());
    b.extend(deadline.to_be_bytes());
    b
}
fn poll(g: &str, k: &str, wait: u32, out: u64) -> Vec<u8> {
    let mut b = Vec::new();
    text(&mut b, g);
    text(&mut b, k);
    b.extend(wait.to_be_bytes());
    b.extend(out.to_be_bytes());
    b.extend(0u64.to_be_bytes());
    b.extend(4096u32.to_be_bytes());
    b
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        assert!(self.0.len() >= n);
        let b = self.0[..n].to_vec();
        self.0 = &self.0[n..];
        b
    }
    fn byte(&mut self) -> u8 {
        self.bytes(1)[0]
    }
    fn int(&mut self) -> u32 {
        u32::from_be_bytes(
            self.bytes(4)
                .try_into()
                .unwrap_or_else(|_| panic!("integer")),
        )
    }
    fn long(&mut self) -> u64 {
        u64::from_be_bytes(
            self.bytes(8)
                .try_into()
                .unwrap_or_else(|_| panic!("integer")),
        )
    }
    fn blob(&mut self) -> Vec<u8> {
        let n = self.int() as usize;
        self.bytes(n)
    }
    fn text(&mut self) -> String {
        String::from_utf8(self.blob()).unwrap_or_else(|e| panic!("UTF8: {e}"))
    }
}
fn completed(reply: &[u8], g: &str, k: &str, code: u32) -> (Vec<u8>, Vec<u8>) {
    let mut r = Reader(reply);
    assert_eq!(r.byte(), 0, "{reply:?}");
    assert_eq!(r.text(), g);
    assert_eq!(r.text(), k);
    assert_eq!(r.byte(), 4);
    assert_eq!(r.byte(), 1);
    assert_eq!(r.int(), code);
    let mut streams = Vec::new();
    for _ in 0..2 {
        let from = r.long();
        let next = r.long();
        let bytes = r.long();
        let content = r.blob();
        assert_eq!(next - from, content.len() as u64);
        assert!(next <= bytes);
        assert!(r.byte() <= 1);
        assert!(r.byte() <= 1);
        streams.push(content);
    }
    assert!(r.0.is_empty());
    (streams.remove(0), streams.remove(0))
}
#[test]
fn runs_persist_and_retry_wait_does_not_resubmit() {
    let s = Sandbox::new();
    let g = s.prepare();
    let (k, expiry) = key();
    let script = "printf hello; printf error >&2; exit 7";
    let reply = s.call(2, &start(&g, &k, expiry, script, 3000, 0));
    let (out, err) = completed(&reply, &g, &k, 7);
    assert_eq!(out, b"hello");
    assert_eq!(err, b"error");
    let again = s.call(2, &start(&g, &k, expiry, script, 1, 0));
    completed(&again, &g, &k, 7);
    let changed = s.call(2, &start(&g, &k, expiry, "exit 0", 1, 0));
    let mut r = Reader(&changed);
    assert_eq!(r.byte(), 1);
    assert_eq!(r.text(), "IDEMPOTENCY_CONFLICT");
    let page = s.call(3, &poll(&g, &k, 0, 2));
    let (out, err) = completed(&page, &g, &k, 7);
    assert_eq!(out, b"llo");
    assert_eq!(err, b"error");
}
#[test]
fn independent_calls_observe_a_detached_run() {
    let s = Sandbox::new();
    let g = s.prepare();
    let (k, expiry) = key();
    let reply = s.call(2, &start(&g, &k, expiry, "sleep 0.2; printf done", 0, 0));
    assert_eq!(reply[0], 0);
    let result = s.call(3, &poll(&g, &k, 3000, 0));
    let (out, _) = completed(&result, &g, &k, 0);
    assert_eq!(out, b"done");
}
