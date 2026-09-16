//! Directory sync: which destination `--delete` may prune, and the `rsync` argv.

use super::args::{check_destination, local_arg, remote_spec, validate_remote_path};
use super::changes::CHANGE_MARKER;
use super::{Refusal, reject};
use std::path::Path;

/// Decides whether a directory-sync destination is safe to write to — and, with
/// `--delete`, safe to prune. The rules are few and mechanical.
pub fn reject_sync_target(destination: &str, delete: bool) -> Result<(), Refusal> {
    validate_remote_path(destination)?;
    let cleaned = clean_remote_path(destination);
    check_destination(&cleaned)?;
    if matches!(cleaned.as_str(), "." | "./" | ".." | "../") {
        return Err(reject(format!(
            "sync destination {cleaned:?} depends on the remote working directory; use a path under the home directory or an absolute path"
        )));
    }
    if !delete {
        return Ok(());
    }
    if cleaned == "~" || cleaned == "~/" {
        return Err(reject(format!(
            "--delete refuses an entire home directory ({cleaned:?}); choose a subdirectory"
        )));
    }
    if let Some(rest) = cleaned.strip_prefix('~') {
        if !rest.contains('/') {
            return Err(reject(format!(
                "--delete refuses an entire home directory ({cleaned:?}); choose a subdirectory"
            )));
        }
    }
    if Path::new(&cleaned).is_absolute() && cleaned.trim_matches('/').split('/').count() == 1 {
        return Err(reject(format!(
            "--delete refuses a top-level directory ({cleaned:?}); sync into a subdirectory instead"
        )));
    }
    Ok(())
}

/// Lexically cleans a remote path the way the remote shell's tools treat it:
/// repeated separators collapse, `.` disappears, and `..` cancels the component
/// before it. A remote path is POSIX on every host, so this is deliberately not
/// the platform's own path rules (a host is never a Windows path).
///
/// The cleaning happens before the `--delete` rules are judged, because
/// `"/tmp/../"` names the filesystem root however it was typed.
fn clean_remote_path(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let rooted = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => match parts.last() {
                Some(last) if *last != ".." => {
                    parts.pop();
                }
                _ if !rooted => parts.push(".."),
                _ => {}
            },
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if rooted {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// One directory sync. The direction decides which endpoint the destination
/// guard judges, because that is the side `--delete` can prune.
pub struct SyncRequest {
    pub host: String,
    pub source: String,
    pub destination: String,
    pub delete: bool,
    pub dry_run: bool,
    pub excludes: Vec<String>,
}

/// The rsync argv for one local→remote sync, the remote spec it names, and
/// whether the transfer still carries rhost's own options.
pub struct SyncPlan {
    pub args: Vec<String>,
    /// The endpoint the plan reports, with a trailing slash: the *contents* of a
    /// source are transferred, and the reported value says so.
    pub source: String,
    pub destination: String,
    pub multiplexed: bool,
}

fn transfer_options(request: &SyncRequest) -> Result<Vec<String>, Refusal> {
    let mut args = vec![
        "--recursive".to_string(),
        // --times preserves mtimes so a later sync is incremental rather than
        // "the whole tree changed"; ownership and permissions are explicit
        // rather than left to a default that would need root remotely.
        "--times".to_string(),
        "--no-owner".to_string(),
        "--no-group".to_string(),
        "--no-perms".to_string(),
    ];
    if request.dry_run {
        args.push("--dry-run".to_string());
    }
    if request.delete {
        args.push("--delete".to_string());
    }
    for pattern in &request.excludes {
        if pattern.contains('\n') || pattern.contains('\0') {
            return Err(reject(format!(
                "exclude pattern {pattern:?} contains a newline or NUL"
            )));
        }
        args.push("--exclude".to_string());
        args.push(pattern.clone());
    }
    args.push("--itemize-changes".to_string());
    args.push(format!("--out-format={CHANGE_MARKER}%i|%n"));
    Ok(args)
}

/// Local directory → remote directory.
pub fn push_args(request: &SyncRequest, ssh_options: &[String]) -> Result<SyncPlan, Refusal> {
    reject_sync_target(&request.destination, request.delete)?;
    let source = local_arg(&request.source)?;
    let mut args = transfer_options(request)?;
    let (shell, multiplexed) = remote_shell(ssh_options);
    args.push("-e".to_string());
    args.push(shell);
    // The source's *contents* are transferred; the trailing slash is added here
    // because "src" and "src/" mean different things to rsync.
    let source = format!("{}/", source.trim_end_matches('/'));
    args.push(source.clone());
    let remote = remote_spec(&request.host, &request.destination);
    args.push(remote.clone());
    Ok(SyncPlan {
        args,
        source,
        destination: remote,
        multiplexed,
    })
}

/// Remote directory → local directory. The risk is reversed here: `--delete`
/// threatens *local* files, so the destination guard judges the local path.
pub fn pull_args(request: &SyncRequest, ssh_options: &[String]) -> Result<SyncPlan, Refusal> {
    reject_sync_target(&request.destination, request.delete)?;
    let destination = local_arg(&request.destination)?;
    let mut args = transfer_options(request)?;
    let (shell, multiplexed) = remote_shell(ssh_options);
    args.push("-e".to_string());
    args.push(shell);
    let source = remote_spec(
        &request.host,
        &format!("{}/", request.source.trim_end_matches('/')),
    );
    args.push(source.clone());
    args.push(destination.clone());
    Ok(SyncPlan {
        args,
        source,
        destination,
        multiplexed,
    })
}

/// Renders rsync's `-e` value and reports whether it still carries rhost's
/// options. rsync splits its own `-e` string with shell-like rules, so an option
/// containing whitespace is single-quoted instead of dropped; an option that
/// cannot be represented that way reports `false`, and the transfer runs over a
/// fresh connection rather than mangled argv.
fn remote_shell(ssh_options: &[String]) -> (String, bool) {
    let mut quoted: Vec<String> = Vec::with_capacity(ssh_options.len() + 1);
    for option in ssh_options {
        if option.contains('\n') || option.contains('\0') {
            return ("ssh".to_string(), false);
        }
        match quote_rsh(option) {
            Some(value) => quoted.push(value),
            None => return ("ssh".to_string(), false),
        }
    }
    if quoted.is_empty() {
        return ("ssh".to_string(), false);
    }
    (format!("ssh {}", quoted.join(" ")), true)
}

fn quote_rsh(option: &str) -> Option<String> {
    if !option.contains([' ', '\t']) {
        return Some(option.to_string());
    }
    if option.contains('\'') {
        return None;
    }
    Some(format!("'{option}'"))
}
