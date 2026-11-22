//! Detached userspace forwarder supervisor over an RH02 helper channel.
use super::{
    Fault, Request,
    record::{DestinationEvidence, Kind},
    relay::{handle_client, load_evidence},
};
use crate::{
    helper::{Helper, mux::Channel},
    transport::{Client, Config},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::Permissions,
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::TcpListener,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8},
        mpsc,
    },
    time::{Duration, Instant},
};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) enum WorkerControl {
    Alive(DestinationEvidence),
    SocketAbsent,
    Uncertain(String),
}

#[derive(Serialize, Deserialize)]
struct WorkerSpec {
    ssh_bin: String,
    control_path: String,
    control_persist: String,
    connect_timeout_ms: u64,
    batch_mode: bool,
    log_level: String,
    host: String,
    kind: Kind,
    listen: String,
    destination: Option<String>,
    socket: PathBuf,
}

pub(super) fn start(client: &Client, socket: &Path, request: &Request<'_>) -> Result<(), Fault> {
    let cfg = client.config();
    let spec = WorkerSpec {
        ssh_bin: cfg.ssh_bin.clone(),
        control_path: cfg.control_path.clone(),
        control_persist: cfg.control_persist.clone(),
        connect_timeout_ms: u64::try_from(cfg.connect_timeout.as_millis()).unwrap_or(15_000),
        batch_mode: cfg.batch_mode,
        log_level: cfg.log_level.clone(),
        host: request.host.to_string(),
        kind: request.kind,
        listen: request.listen.clone(),
        destination: request.destination.clone(),
        socket: socket.to_path_buf(),
    };
    let payload = serde_json::to_vec(&spec).map_err(|e| Fault::Invalid(e.to_string()))?;
    let exe = std::env::current_exe().map_err(|e| Fault::Transport(e.to_string()))?;
    let mut child = Command::new(exe)
        .arg("--internal-tunnel-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Fault::Transport(e.to_string()))?;
    if let Some(mut stdin) = child.stdin.take()
        && let Err(error) = stdin.write_all(&payload)
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Fault::Transport(error.to_string()));
    }
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Fault::Uncertain(
            "tunnel worker is missing stdout pipe".into(),
        ));
    };
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let res = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = tx.send(res);
    });
    let budget = cfg.connect_timeout + Duration::from_secs(10);
    match rx.recv_timeout(budget) {
        Ok(Ok(line)) if line.trim() == "OK" => match check(socket) {
            WorkerControl::Alive(_) => Ok(()),
            WorkerControl::SocketAbsent => Err(Fault::Transport(
                "tunnel worker exited before accepting control checks".into(),
            )),
            WorkerControl::Uncertain(reason) => Err(Fault::Uncertain(reason)),
        },
        Ok(Ok(line)) => {
            let _ = child.wait();
            let message = line
                .trim()
                .strip_prefix("ERR ")
                .unwrap_or_else(|| line.trim());
            let detail = if message.is_empty() {
                "tunnel worker failed during startup"
            } else {
                message
            };
            Err(Fault::Transport(detail.to_string()))
        }
        Ok(Err(error)) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(Fault::Transport(error.to_string()))
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(Fault::Uncertain(
                "timed out waiting for helper tunnel startup".into(),
            ))
        }
    }
}

pub(super) fn check(socket: &Path) -> WorkerControl {
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            return WorkerControl::SocketAbsent;
        }
        Err(error) => return WorkerControl::Uncertain(error.to_string()),
    };
    let _ = stream.set_read_timeout(Some(CONTROL_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CONTROL_TIMEOUT));
    if let Err(error) = stream.write_all(b"CHECK\n") {
        return WorkerControl::Uncertain(error.to_string());
    }
    let mut line = String::new();
    match BufReader::new(stream).read_line(&mut line) {
        Ok(0) => WorkerControl::SocketAbsent,
        Ok(_) => {
            let trimmed = line.trim();
            if trimmed == "DEAD" {
                WorkerControl::SocketAbsent
            } else if let Some(ev) = trimmed
                .strip_prefix("ALIVE ")
                .and_then(DestinationEvidence::parse)
            {
                WorkerControl::Alive(ev)
            } else {
                WorkerControl::Uncertain(format!("unexpected tunnel control answer: {trimmed}"))
            }
        }
        Err(error) => WorkerControl::Uncertain(error.to_string()),
    }
}

pub(super) fn stop(socket: &Path) -> Result<(), String> {
    match UnixStream::connect(socket) {
        Ok(mut stream) => {
            let _ = stream.set_read_timeout(Some(CONTROL_TIMEOUT));
            let _ = stream.set_write_timeout(Some(CONTROL_TIMEOUT));
            stream
                .write_all(b"STOP\n")
                .map_err(|e| format!("failed sending stop to tunnel worker: {e}"))?;
            let mut line = String::new();
            let _ = BufReader::new(stream).read_line(&mut line);
        }
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(());
        }
        Err(error) => return Err(error.to_string()),
    }
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    loop {
        if matches!(check(socket), WorkerControl::SocketAbsent) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("tunnel worker did not stop cleanly".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub fn serve_worker() -> ExitCode {
    let sighup = Arc::new(AtomicBool::new(false));
    let _ = signal_hook::flag::register(signal_hook::consts::SIGHUP, sighup);
    let mut raw = Vec::new();
    if std::io::stdin().read_to_end(&mut raw).is_err() {
        report("ERR failed reading tunnel worker specification");
        return ExitCode::FAILURE;
    }
    let spec: WorkerSpec = match serde_json::from_slice(&raw) {
        Ok(spec) => spec,
        Err(error) => {
            report(&format!("ERR invalid tunnel worker spec: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let client = Client::new(Config {
        ssh_bin: spec.ssh_bin.clone(),
        control_path: spec.control_path.clone(),
        control_persist: spec.control_persist.clone(),
        connect_timeout: Duration::from_millis(spec.connect_timeout_ms.max(1000)),
        batch_mode: spec.batch_mode,
        log_level: spec.log_level.clone(),
    });
    let channel = match Helper::new(&client, &spec.host).open_mux(true) {
        Ok(channel) => Arc::new(channel),
        Err(error) => {
            report(&format!("ERR {}", error.message));
            return ExitCode::FAILURE;
        }
    };
    let tcp_listener = match TcpListener::bind(&spec.listen) {
        Ok(listener) => listener,
        Err(error) => {
            report(&format!("ERR bind {} failed: {error}", spec.listen));
            return ExitCode::FAILURE;
        }
    };
    let ctl_listener = match UnixListener::bind(&spec.socket) {
        Ok(listener) => listener,
        Err(error) => {
            report(&format!("ERR control socket bind failed: {error}"));
            return ExitCode::FAILURE;
        }
    };
    let _ = std::fs::set_permissions(&spec.socket, Permissions::from_mode(0o600));
    report("OK");

    let evidence = Arc::new(AtomicU8::new(0));
    let ctl_channel = Arc::clone(&channel);
    let ctl_evidence = Arc::clone(&evidence);
    let ctl_socket = spec.socket.clone();
    std::thread::spawn(move || {
        serve_control(ctl_listener, &ctl_channel, &ctl_evidence, &ctl_socket)
    });

    let watch_channel = Arc::clone(&channel);
    let watch_socket = spec.socket.clone();
    std::thread::spawn(move || {
        while watch_channel.is_alive() {
            std::thread::sleep(Duration::from_millis(250));
        }
        let _ = std::fs::remove_file(&watch_socket);
        std::process::exit(0);
    });

    let destination = spec
        .destination
        .as_deref()
        .and_then(|value| super::split_host_port(value).map(|(h, p)| (h.to_string(), p)));
    for stream in tcp_listener.incoming() {
        let Ok(stream) = stream else {
            break;
        };
        let chan = Arc::clone(&channel);
        let ev = Arc::clone(&evidence);
        let kind = spec.kind;
        let dest = destination.clone();
        std::thread::spawn(move || handle_client(stream, &chan, kind, dest, &ev));
    }
    let _ = std::fs::remove_file(&spec.socket);
    ExitCode::SUCCESS
}

fn report(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn serve_control(listener: UnixListener, channel: &Channel, evidence: &AtomicU8, socket: &Path) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            break;
        };
        let _ = stream.set_read_timeout(Some(CONTROL_TIMEOUT));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        match line.trim() {
            "CHECK" => {
                let answer = if channel.is_alive() {
                    format!("ALIVE {}\n", load_evidence(evidence).as_str())
                } else {
                    "DEAD\n".to_string()
                };
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
            }
            "STOP" => {
                let _ = stream.write_all(b"OK\n");
                let _ = stream.flush();
                let _ = std::fs::remove_file(socket);
                std::process::exit(0);
            }
            _ => {}
        }
    }
}
