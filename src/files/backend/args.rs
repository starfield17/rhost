//! The two operands of a single-file copy, and the `scp` argv built from them.

use super::{Refusal, invalid, reject};
use std::path::Path;

/// Rejects a pair of outer shell quotes while preserving quote characters
/// anywhere inside the name: quotes belong to the caller's shell, never to the
/// path (FS-001).
pub fn validate_remote_path(path: &str) -> Result<(), Refusal> {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return Err(invalid(format!(
                "remote path {path:?} includes outer shell quotes; quote the argument without making quotes part of the path"
            )));
        }
    }
    Ok(())
}

/// Renders the `host:path` form scp and rsync understand. The host is whatever
/// the user named and is never rewritten; IPv6 literals are bracketed so the
/// first colon stays a separator.
pub fn remote_spec(host: &str, path: &str) -> String {
    let mut host = host.to_string();
    if host.contains(':') && !host.contains('[') {
        host = match host.rfind('@') {
            Some(at) => format!("{}[{}]", &host[..at + 1], &host[at + 1..]),
            None => format!("[{host}]"),
        };
    }
    format!("{host}:{path}")
}

/// Splits the `host:path` form with the rule scp and rsync apply: a colon before
/// any slash means a host follows. A spec is only ever parsed here to validate
/// the path part; the host travels to OpenSSH untouched.
pub fn split_remote_spec(argument: &str) -> Option<(&str, &str)> {
    let mut at = argument.find(':');
    if let Some(bracket) = argument.find('[') {
        if at.is_none() || bracket < at.unwrap_or(usize::MAX) {
            let rest = &argument[bracket..];
            let end = rest.find(']')?;
            let after = bracket + end + 1;
            if argument[after..].starts_with(':') {
                at = Some(after);
            } else {
                return None;
            }
        }
    }
    let at = at?;
    if at == 0 || argument[..at].contains('/') {
        return None;
    }
    Some((&argument[..at], &argument[at + 1..]))
}

/// Rewrites a local path so neither tool can mistake it for a remote spec: both
/// split on the first colon and both read a leading dash as an option. The
/// rewrite is only ever a `./` prefix, which names the same file.
pub fn local_arg(path: &str) -> Result<String, Refusal> {
    if path.is_empty() {
        return Err(reject("empty local path"));
    }
    if path.contains('\n') || path.contains('\0') {
        return Err(reject(format!(
            "local path {path:?} contains a newline or NUL"
        )));
    }
    if Path::new(path).is_absolute()
        || matches!(path, "." | "..")
        || path.starts_with("./")
        || path.starts_with("../")
    {
        return Ok(path.to_string());
    }
    if path.starts_with('-') || path.contains(':') {
        return Ok(format!("./{path}"));
    }
    Ok(path.to_string())
}

/// The destination shapes that must never reach a remote tool: empty, the
/// filesystem root, a glob, or control characters.
pub(super) fn check_destination(path: &str) -> Result<(), Refusal> {
    if let Some((_, remote)) = split_remote_spec(path) {
        return check_destination(remote);
    }
    if path.trim().is_empty() {
        return Err(reject("empty destination path"));
    }
    if path.contains('\n') || path.contains('\0') {
        return Err(reject(format!(
            "destination path {path:?} contains a newline or NUL"
        )));
    }
    if path.contains(['*', '?', '[']) {
        return Err(reject(format!(
            "destination path {path:?} contains a glob character: the remote shell would expand it"
        )));
    }
    if path.trim_end_matches('/').is_empty() {
        return Err(reject(format!(
            "destination path {path:?} is the filesystem root"
        )));
    }
    Ok(())
}

/// Checks the arguments of one single-file copy. rhost does not guess what a
/// remote path *means*: it refuses where being wrong is destructive or the tool
/// would misparse the argument, and otherwise gets out of the way.
pub fn validate_transfer_paths(source: &str, destination: &str) -> Result<(), Refusal> {
    if source.trim().is_empty() {
        return Err(reject("empty source path"));
    }
    if source.contains('\n') || source.contains('\0') {
        return Err(reject(format!(
            "source path {source:?} contains a newline or NUL"
        )));
    }
    if let Some((_, remote)) = split_remote_spec(source) {
        validate_remote_path(remote)?;
        if remote.contains(['*', '?', '[']) {
            return Err(reject(format!(
                "source path {remote:?} contains a glob; fs get copies exactly one file"
            )));
        }
    }
    if let Some((_, remote)) = split_remote_spec(destination) {
        validate_remote_path(remote)?;
    }
    check_destination(destination)
}

/// Builds the scp argv for one copy in either direction. `source` and
/// `destination` must already be final scp form: a local side prepared with
/// [`local_arg`], a remote one with [`remote_spec`]. `-q` keeps scp's progress
/// bar off stdout, which `--json` must never mix with data (WIRE-002).
pub fn scp_args(
    source: &str,
    destination: &str,
    ssh_options: &[String],
) -> Result<Vec<String>, Refusal> {
    let mut args = ssh_options.to_vec();
    args.push("-q".to_string());
    args.push(transfer_arg(source)?);
    args.push(transfer_arg(destination)?);
    Ok(args)
}

/// Refuses an operand that cannot be passed safely rather than rewriting it: an
/// empty one, one carrying control characters, or one a tool reads as an option.
fn transfer_arg(argument: &str) -> Result<String, Refusal> {
    if argument.is_empty() {
        return Err(reject("empty path"));
    }
    if argument.contains('\n') || argument.contains('\0') {
        return Err(reject(format!(
            "path {argument:?} contains a newline or NUL"
        )));
    }
    if argument.starts_with('-') {
        return Err(reject(format!(
            "path {argument:?} starts with a dash; prefix it with ./"
        )));
    }
    Ok(argument.to_string())
}
