//! One file, one copy, in either direction.
//!
//! `scp` does the plain copy; `--resume`/`--checksum` take the rsync route with
//! a SHA-256 computed at both ends, because being told when the two ends
//! disagree is the only reason to ask for verification.

use super::super::Error;
use super::super::exec;
use super::remote::{first_line, master_alive, remote_has_rsync, run_remote, validation};
use super::{GetOptions, PutOptions, Transfer};
use crate::fileops::runner::{Stop, Tools};
use crate::fileops::{self, runner};
use crate::shell;
use crate::transport::openssh::Client;
use sha2::{Digest, Sha256};
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

fn stat_local_file(path: &Path, verb: &str) -> Result<u64, Error> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        Error::new(
            "CONFIG_INVALID",
            format!("cannot read {}: {error}", path.display()),
        )
    })?;
    if metadata.is_dir() {
        return Err(Error::new(
            "CONFIG_INVALID",
            format!("fs {verb} copies one file; use fs sync for a directory"),
        ));
    }
    if !metadata.is_file() {
        return Err(Error::new(
            "CONFIG_INVALID",
            format!("fs {verb} needs a regular file, not {}", path.display()),
        ));
    }
    Ok(metadata.len())
}

fn local_sha256(path: &Path) -> Result<(String, u64), Error> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| Error::new("TRANSFER_FAILED", error.to_string()))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| Error::new("TRANSFER_FAILED", error.to_string()).retryable())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok((hex(&hasher.finalize()), size))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The file a copy landed on: the named path, or the same base name inside it
/// when the name is an existing directory. Reporting the directory would leave
/// the caller to guess which file appeared.
pub(crate) fn remote_file_of(
    client: &Client,
    host: &str,
    remote_path: &str,
    base: &str,
    timeout: Duration,
) -> Result<String, Error> {
    let probe = format!(
        "p={}; if [ -d \"$p\" ]; then printf '%s/%s' \"$p\" {}; else printf '%s' \"$p\"; fi",
        shell::path_quote(remote_path),
        shell::quote(base)
    );
    let command = format!("sh -c {}", shell::quote(&probe));
    let (code, stdout, _) = run_remote(
        client,
        host,
        &command,
        None,
        timeout,
        exec::DEFAULT_JSON_CAPTURE,
    )?;
    let stdout = stdout.trim().to_string();
    if code != 0 || stdout.is_empty() {
        return Err(Error::new(
            "TRANSFER_FAILED",
            "cannot resolve the remote destination of the copy",
        ));
    }
    Ok(stdout)
}

fn ensure_remote_parent(
    client: &Client,
    host: &str,
    remote_path: &str,
    timeout: Duration,
) -> Result<(), Error> {
    let Some(command) = fileops::remote_parent_command(remote_path) else {
        return Ok(());
    };
    let (code, _, stderr) = run_remote(
        client,
        host,
        &command,
        None,
        timeout,
        exec::DEFAULT_JSON_CAPTURE,
    )?;
    if code != 0 {
        return Err(Error::new(
            "TRANSFER_FAILED",
            format!(
                "could not create the remote parent directory: {}",
                first_line(&stderr)
            ),
        ));
    }
    Ok(())
}

/// Runs one local transfer tool, mapping what happened onto the taxonomy. A tool
/// that is not installed here is not the remote's fault and not retryable.
pub(crate) fn run_tool(
    tools: &Tools,
    program: &str,
    args: &[String],
    timeout: Option<Duration>,
    stop: Stop<'_>,
) -> Result<runner::Outcome, Error> {
    if let Err(error) = crate::config::ensure_control_dir() {
        return Err(Error::new("CONFIG_INVALID", error.to_string()));
    }
    let args = if program == tools.rsync {
        tools.safe_rsync_paths(args, stop)
    } else {
        args.to_vec()
    };
    let outcome = runner::run(
        program,
        &args,
        timeout.unwrap_or(runner::DEFAULT_TIMEOUT),
        stop,
    );
    if let Some(error) = outcome.spawn_error.as_ref() {
        return Err(if error.kind() == std::io::ErrorKind::NotFound {
            Error::new(
                "TRANSFER_FAILED",
                format!("{program} is not installed locally"),
            )
        } else {
            Error::new(
                "TRANSFER_FAILED",
                format!("cannot run {program} on this machine: {error}"),
            )
        });
    }
    if outcome.timed_out {
        return Err(Error::new(
            "REMOTE_COMMAND_TIMEOUT",
            "the transfer exceeded its timeout; remote transfer state is unknown",
        ));
    }
    if let Some(signal) = outcome.cancelled {
        return Err(Error::new(
            "REMOTE_COMMAND_CANCELLED",
            format!(
                "the transfer was cancelled by {}; remote transfer state is unknown",
                match signal {
                    crate::domain::CancelSignal::Int => "SIGINT",
                    crate::domain::CancelSignal::Term => "SIGTERM",
                }
            ),
        ));
    }
    if outcome.overflow {
        return Err(Error::new(
            "TRANSFER_FAILED",
            format!("{program} produced more output than rhost carries back"),
        ));
    }
    if outcome.exit != Some(0) {
        return Err(Error::new(
            "TRANSFER_FAILED",
            format!(
                "{program} could not copy the file: {}",
                outcome.diagnostic()
            ),
        )
        .retryable());
    }
    Ok(outcome)
}

/// Copies one local file to a remote path.
pub fn put(
    client: &Client,
    tools: &Tools,
    options: &PutOptions<'_>,
    stop: Stop<'_>,
) -> Result<Transfer, Error> {
    fileops::validate_remote_path(options.remote).map_err(validation)?;
    let local = fileops::local_arg(options.local_path).map_err(validation)?;
    let destination = fileops::remote_spec(options.host, options.remote);
    fileops::validate_transfer_paths(&local, &destination).map_err(validation)?;
    let size = stat_local_file(Path::new(&local), "put")?;
    let timeout = options.timeout.unwrap_or(runner::DEFAULT_TIMEOUT);
    if options.parents {
        ensure_remote_parent(client, options.host, options.remote, timeout)?;
    }
    if options.resume || options.checksum {
        return verified(
            client,
            tools,
            options.host,
            &local,
            options.remote,
            false,
            options.resume,
            timeout,
            stop,
        );
    }
    let base = Path::new(&local)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let effective = remote_file_of(client, options.host, options.remote, &base, timeout)?;
    let argv =
        fileops::scp_args(&local, &destination, &client.ssh_options()).map_err(validation)?;
    let outcome = run_tool(tools, &tools.scp, &argv, options.timeout, stop)?;
    Ok(Transfer {
        source: local,
        destination: fileops::remote_spec(options.host, &effective),
        backend: "scp",
        size,
        multiplexed: master_alive(client, options.host),
        duration_ms: outcome.duration.as_millis() as u64,
        checksum_verified: false,
        resume_enabled: false,
    })
}

/// Copies one remote file to a local path.
pub fn get(
    client: &Client,
    tools: &Tools,
    options: &GetOptions<'_>,
    stop: Stop<'_>,
) -> Result<Transfer, Error> {
    fileops::validate_remote_path(options.remote).map_err(validation)?;
    let timeout = options.timeout.unwrap_or(runner::DEFAULT_TIMEOUT);
    if options.resume || options.checksum {
        let local = fileops::local_arg(options.local_path).map_err(validation)?;
        return verified(
            client,
            tools,
            options.host,
            &local,
            options.remote,
            true,
            options.resume,
            timeout,
            stop,
        );
    }
    let source = fileops::remote_spec(options.host, options.remote);
    let local = fileops::local_arg(options.local_path).map_err(validation)?;
    fileops::validate_transfer_paths(&source, &local).map_err(validation)?;
    let argv = fileops::scp_args(&source, &local, &client.ssh_options()).map_err(validation)?;
    let outcome = run_tool(tools, &tools.scp, &argv, options.timeout, stop)?;
    // scp writes into an existing directory rather than replacing it, so the
    // effective destination is the file it created.
    let mut effective = local.clone();
    if std::fs::metadata(&local).is_ok_and(|metadata| metadata.is_dir()) {
        effective = Path::new(&local)
            .join(fileops::path_base(options.remote))
            .to_string_lossy()
            .to_string();
    }
    let size = std::fs::metadata(&effective)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    Ok(Transfer {
        source,
        destination: effective,
        backend: "scp",
        size,
        multiplexed: master_alive(client, options.host),
        duration_ms: outcome.duration.as_millis() as u64,
        checksum_verified: false,
        resume_enabled: false,
    })
}

/// The `--resume` / `--checksum` route. The argument list is long because a
/// verified copy really does need both endpoints, both flags and a deadline.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verified(
    client: &Client,
    tools: &Tools,
    host: &str,
    local: &str,
    remote_path: &str,
    download: bool,
    resume: bool,
    timeout: Duration,
    stop: Stop<'_>,
) -> Result<Transfer, Error> {
    let (source, destination) = if download {
        (fileops::remote_spec(host, remote_path), local.to_string())
    } else {
        (local.to_string(), fileops::remote_spec(host, remote_path))
    };
    fileops::validate_transfer_paths(&source, &destination).map_err(validation)?;
    remote_has_rsync(client, host)?;

    let mut args = vec!["--times".to_string(), "--checksum".to_string()];
    let mut ssh = vec!["ssh".to_string()];
    for option in client.ssh_options() {
        ssh.push(shell::quote(&option));
    }
    args.push("-e".to_string());
    args.push(ssh.join(" "));
    if resume {
        // A dedicated partial directory, so an interrupted rhost transfer never
        // mixes with a partial left by another invocation.
        args.push("--partial-dir=.rhost-partial".to_string());
    }
    args.push("--".to_string());
    args.push(source.clone());
    args.push(destination.clone());

    let outcome = run_tool(tools, &tools.rsync, &args, Some(timeout), stop)?;

    // Both tools write to the *named* destination, which for a directory means
    // the file inside it. The reported endpoints are the ones that actually hold
    // the bytes, and the local one is also the file that gets hashed here — a
    // remote spec is never a local path.
    let (local_file, effective_destination, remote_file) = if download {
        let named = Path::new(local);
        let file = match std::fs::metadata(named) {
            Ok(metadata) if metadata.is_dir() => named.join(fileops::path_base(remote_path)),
            _ => named.to_path_buf(),
        };
        let file = file.to_string_lossy().to_string();
        (file.clone(), file, remote_path.to_string())
    } else {
        let base = Path::new(local)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        let remote_file = remote_file_of(client, host, remote_path, &base, timeout)?;
        (
            local.to_string(),
            fileops::remote_spec(host, &remote_file),
            remote_file,
        )
    };

    let (want, size) = local_sha256(Path::new(&local_file))?;
    let got = remote_sha256(client, host, &remote_file, timeout)?;
    if got != want {
        return Err(Error::new(
            "TRANSFER_FAILED",
            format!("end-to-end SHA-256 verification failed for {remote_file}"),
        )
        .retryable());
    }
    Ok(Transfer {
        source,
        destination: effective_destination,
        backend: "rsync",
        size,
        multiplexed: master_alive(client, host),
        duration_ms: outcome.duration.as_millis() as u64,
        checksum_verified: true,
        resume_enabled: resume,
    })
}

/// Asks the remote for a digest. 127 is the *tool* being absent, which is a
/// capability answer and not a checksum mismatch: telling those apart is what
/// lets an agent fix the right problem.
pub(crate) fn remote_sha256(
    client: &Client,
    host: &str,
    remote_path: &str,
    timeout: Duration,
) -> Result<String, Error> {
    let command = format!("sha256sum -- {}", shell::path_quote(remote_path));
    let (code, stdout, _) = run_remote(
        client,
        host,
        &command,
        None,
        timeout,
        exec::DEFAULT_JSON_CAPTURE,
    )?;
    if code == 127 {
        return Err(Error::new(
            "REMOTE_DEPENDENCY_MISSING",
            "verified transfer needs sha256sum on the remote host",
        ));
    }
    let digest = stdout.split_whitespace().next().unwrap_or("").to_string();
    if code != 0 || digest.is_empty() {
        return Err(Error::new(
            "TRANSFER_FAILED",
            format!("cannot read the remote checksum of {remote_path}"),
        )
        .retryable());
    }
    Ok(digest)
}
