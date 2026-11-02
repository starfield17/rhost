//! Status-only observation and observer-lifetime acceptance.
use super::*;

#[test]
fn watch_reports_counters_without_transferring_log_bodies() {
    let s = Sandbox::new();
    let g = s.prepare();
    let (k, expiry) = key();
    s.call(
        2,
        &start(
            &g,
            &k,
            expiry,
            "printf 'long retained stdout'; printf 'stderr' >&2; exit 7",
            3000,
            0,
        ),
    );
    // Leader completion precedes final draining: wait for closed stream evidence.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let page = s.call(3, &poll(&g, &k, 0, 0));
        let mut r = Reader(&page);
        assert_eq!(r.byte(), 0);
        assert_eq!(r.text(), g);
        assert_eq!(r.text(), k);
        assert_eq!(r.byte(), 4);
        assert_eq!(r.byte(), 1);
        assert_eq!(r.int(), 7);
        let mut closed = true;
        for _ in 0..2 {
            r.long();
            r.long();
            r.long();
            r.blob();
            closed &= r.byte() == 0;
            assert_eq!(r.byte(), 0);
        }
        if closed {
            break;
        }
        assert!(std::time::Instant::now() < until, "streams did not close");
    }
    let mut body = Vec::new();
    text(&mut body, &g);
    text(&mut body, &k);
    body.extend(3000u32.to_be_bytes());
    let reply = s.call(9, &body);
    let mut r = Reader(&reply);
    assert_eq!(r.byte(), 0);
    assert_eq!(r.text(), g);
    assert_eq!(r.text(), k);
    assert_eq!(r.byte(), 4);
    assert_eq!(r.byte(), 1);
    assert_eq!(r.int(), 7);
    for expected in [20, 6] {
        assert_eq!(r.long(), expected);
        assert_eq!(r.long(), expected);
        assert!(r.byte() <= 1, "stream-open flag must be Boolean");
        assert!(r.byte() <= 1, "incomplete flag must be Boolean");
    }
    assert!(r.0.is_empty(), "watch contains an unexpected log body");
}

#[test]
fn watch_wait_is_bounded_and_observer_death_does_not_stop_work() {
    let s = Sandbox::new();
    let g = s.prepare();
    let (k, expiry) = key();
    let release = s.0.join("release-watch");
    let script = format!(
        "while [ ! -f '{}' ]; do sleep 0.01; done; exit 7",
        release.display()
    );
    s.call(2, &start(&g, &k, expiry, &script, 0, 3000));
    let body = |wait: u32| {
        let mut b = Vec::new();
        text(&mut b, &g);
        text(&mut b, &k);
        b.extend(wait.to_be_bytes());
        b
    };
    let started = std::time::Instant::now();
    let reply = s.call(9, &body(100));
    assert!(started.elapsed() >= std::time::Duration::from_millis(100));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "watch exceeded bounded wait"
    );
    let mut r = Reader(&reply);
    assert_eq!(r.byte(), 0);
    assert_eq!(r.text(), g);
    assert_eq!(r.text(), k);
    assert!(matches!(r.byte(), 1..=3));
    assert_eq!(s.call(9, &body(300001))[0], 1);
    let immediate = std::time::Instant::now();
    assert_eq!(s.call(9, &body(0))[0], 0);
    assert!(immediate.elapsed() < std::time::Duration::from_secs(1));
    let helper = std::env::var("RHOST_HELPER_BIN").unwrap_or_else(|_| panic!("helper"));
    let mut child = Command::new(helper)
        .env("RHOST_REMOTE_STATE", &s.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("observer: {e}"));
    let b = body(300000);
    let mut frame = b"RH01".to_vec();
    frame.extend(9u32.to_be_bytes());
    frame.extend((b.len() as u64).to_be_bytes());
    frame.extend(b);
    child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("stdin"))
        .write_all(&frame)
        .unwrap_or_else(|e| panic!("send: {e}"));
    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(
        child
            .try_wait()
            .unwrap_or_else(|e| panic!("observer state: {e}"))
            .is_none(),
        "watch returned before completion"
    );
    child
        .kill()
        .unwrap_or_else(|e| panic!("kill observer: {e}"));
    child
        .wait()
        .unwrap_or_else(|e| panic!("reap observer: {e}"));
    std::fs::write(release, b"continue").unwrap_or_else(|e| panic!("release: {e}"));
    completed(&s.call(3, &poll(&g, &k, 2000, 0)), &g, &k, 7);
}
