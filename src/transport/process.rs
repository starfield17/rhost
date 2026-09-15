//! Synchronous local-process plumbing for one ssh/scp/rsync invocation.
//!
//! The CLI owns nothing durable, but it does own the child it starts, and that
//! ownership stays reviewable: one main loop decides when a run is over, and the
//! two reader threads only move bytes. There is no async runtime and no shared
//! mutable stream state — bytes travel over channels into the main loop, which
//! is the only code that touches a [`Stream`].

//! `capture` owns the bounded copy of a stream and the sinks that write into
//! one; `run` owns the child, its pipes and the loop that decides when a run is
//! over. Everything they share is named here, so a caller never reaches into
//! either half directly.

mod capture;
mod run;

pub use capture::{Capture, Keep, Recorder, Stream, TAIL_KEEP, Tap};
pub use run::{DRAIN_GRACE, Run, RunFailure, Spec, StdinSource, run};

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io;
    use std::process::Command;
    use std::time::{Duration, Instant};

    struct Collect(Vec<u8>, u64);
    impl Stream for Collect {
        fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
            self.0.extend_from_slice(bytes);
            self.1 += bytes.len() as u64;
            Ok(())
        }
    }

    fn never() -> bool {
        false
    }

    fn spec<'a>(program: &'a str, args: &'a [String], stdin: StdinSource<'a>) -> Spec<'a> {
        Spec {
            program,
            args,
            stdin,
            deadline: None,
            cancelled: &never,
            group: false,
        }
    }

    #[test]
    fn a_tap_can_count_without_keeping_anything() {
        let mut quiet = Capture::new(0, Keep::Nothing);
        quiet.observe(b"lots of bytes");
        assert!(quiet.body().is_empty());
        assert_eq!(quiet.total(), 13);
    }

    #[test]
    fn a_child_run_forwards_both_streams_and_keeps_its_status() {
        let args = vec!["-c".into(), "printf out; printf err >&2; exit 7".into()];
        let quiet = spec("/bin/sh", &args, StdinSource::Closed);
        let mut stdout = Collect(Vec::new(), 0);
        let mut stderr = Collect(Vec::new(), 0);
        let run = run(&quiet, &mut stdout, &mut stderr);
        assert_eq!(run.exit, Some(7));
        assert!(!run.timed_out && !run.cancelled);
        assert!(run.failure.is_none(), "{:?}", run.failure);
        assert_eq!(stdout.0, b"out");
        assert_eq!(stderr.1, 3);
        assert_eq!(stderr.0, b"err");
    }

    #[test]
    fn an_unlaunchable_program_is_reported_not_panicked() {
        let args = Vec::new();
        let quiet = spec("/nonexistent-rhost-tool", &args, StdinSource::Closed);
        let mut stdout = Collect(Vec::new(), 0);
        let mut stderr = Collect(Vec::new(), 0);
        let run = run(&quiet, &mut stdout, &mut stderr);
        assert!(matches!(run.failure, Some(RunFailure::Spawn(_))));
        assert_eq!(run.exit, None);
    }

    #[test]
    fn a_prefix_capture_drops_the_tail_but_still_counts_it() {
        let mut capture = Capture::new(8, Keep::Prefix);
        capture.observe(&[b'y'; 40]);
        assert_eq!(capture.total(), 40);
        assert_eq!(capture.body(), vec![b'y'; 8]);
    }

    #[test]
    fn a_buffered_recorder_keeps_counts() {
        let mut recorder = Recorder::new(0);
        recorder
            .feed(b"abc")
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(recorder.total(), 3);
        assert_eq!(recorder.body(), b"abc");
    }

    #[test]
    fn a_deadline_stops_the_child_without_inventing_a_status() {
        let args = vec!["-c".into(), "sleep 30".into()];
        let start = Instant::now() + Duration::from_millis(200);
        let quiet = Spec {
            program: "/bin/sh",
            args: &args,
            stdin: StdinSource::Closed,
            deadline: Some(start),
            cancelled: &never,
            group: false,
        };
        let mut stdout = Collect(Vec::new(), 0);
        let mut stderr = Collect(Vec::new(), 0);
        let run = run(&quiet, &mut stdout, &mut stderr);
        assert!(run.timed_out);
        assert_eq!(run.exit, None);
        assert!(run.duration < Duration::from_secs(5));
    }

    #[test]
    fn cancellation_stops_the_child() {
        let args = vec!["-c".into(), "sleep 30".into()];
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let armed = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            armed.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        let armed = Spec {
            program: "/bin/sh",
            args: &args,
            stdin: StdinSource::Closed,
            deadline: None,
            cancelled: &move || flag.load(std::sync::atomic::Ordering::SeqCst),
            group: false,
        };
        let mut stdout = Collect(Vec::new(), 0);
        let mut stderr = Collect(Vec::new(), 0);
        let run = run(&armed, &mut stdout, &mut stderr);
        assert!(run.cancelled);
        assert!(!run.timed_out);
    }

    /// The group-kill syntax once worked on BSD `kill` but not GNU `kill`: the
    /// shell died while its pipe-holding child survived. Readiness, rather than
    /// a wall-clock guess, proves the descendant existed before cancellation.
    #[test]
    fn cancellation_stops_a_ready_process_group_and_its_descendant() {
        let root = std::env::temp_dir().join(format!(
            "rhost-process-group-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap_or_else(|error| panic!("fixture: {error}"));
        let pid_file = root.join("descendant-pid");
        let parent_file = root.join("parent-pid");
        let ready = root.join("ready");
        let script = r#"printf '%s' "$$" > "$3"
sh -c 'printf "%s" "$$" > "$1"; printf ready > "$2"; exec sleep 30' child "$1" "$2" &
wait"#;
        let args = vec![
            "-c".to_string(),
            script.to_string(),
            "rhost-process-group-test".to_string(),
            pid_file.to_string_lossy().into_owned(),
            ready.to_string_lossy().into_owned(),
            parent_file.to_string_lossy().into_owned(),
        ];
        let observed = Cell::new(false);
        let cancelled = || {
            let is_ready = ready.exists();
            observed.set(observed.get() || is_ready);
            is_ready
        };
        let grouped = Spec {
            program: "/bin/sh",
            args: &args,
            stdin: StdinSource::Closed,
            deadline: Some(Instant::now() + Duration::from_secs(10)),
            cancelled: &cancelled,
            group: true,
        };
        let mut stdout = Collect(Vec::new(), 0);
        let mut stderr = Collect(Vec::new(), 0);
        let run = run(&grouped, &mut stdout, &mut stderr);
        assert!(observed.get(), "the fixture never reported readiness");
        assert!(run.cancelled && !run.timed_out);

        let mut survivors = Vec::new();
        for file in [&parent_file, &pid_file] {
            let raw = std::fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("read fixture pid: {error}"));
            let pid = raw.trim();
            assert!(!pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()));
            if process_is_running(pid) {
                let _ = Command::new("kill").args(["-KILL", pid]).status();
                survivors.push(pid.to_string());
            }
        }
        assert!(
            survivors.is_empty(),
            "group cancellation left {survivors:?} running"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    fn process_is_running(pid: &str) -> bool {
        let output = match Command::new("ps").args(["-o", "stat=", "-p", pid]).output() {
            Ok(output) => output,
            Err(_) => return true,
        };
        let state = String::from_utf8_lossy(&output.stdout);
        output.status.success()
            && state
                .split_whitespace()
                .any(|value| !value.starts_with('Z'))
    }

    #[test]
    fn a_huge_output_is_bounded_but_fully_counted() {
        let mut capture = Capture::new(16, Keep::PrefixAndMarkerTail);
        for _ in 0..1000 {
            capture.observe(&[b'x'; 64]);
        }
        assert_eq!(capture.total(), 64_000);
        let body = capture.body();
        assert_eq!(body.len(), 16 + TAIL_KEEP);
        assert!(body.starts_with(&[b'x'; 16]));
    }

    #[test]
    fn a_sink_failure_ends_the_run() {
        struct Broken;
        impl Stream for Broken {
            fn feed(&mut self, _: &[u8]) -> io::Result<()> {
                Err(io::Error::other("sink is gone"))
            }
        }
        let args = vec!["-c".into(), "head -c 200000 /dev/zero".into()];
        let quiet = spec("/bin/sh", &args, StdinSource::Closed);
        let mut stdout = Broken;
        let mut stderr = Collect(Vec::new(), 0);
        let run = run(&quiet, &mut stdout, &mut stderr);
        assert!(
            matches!(run.failure, Some(RunFailure::Stream(_))),
            "delivery failure was swallowed"
        );
    }
}
