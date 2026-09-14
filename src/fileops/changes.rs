//! rsync's itemized plan, read back as structured changes.

use super::args::split_remote_spec;

/// Prefixes every itemized plan line so a parser can tell rsync's plan from the
/// warnings, motd and deletion notices the same stdout also carries.
pub const CHANGE_MARKER: &str = "RHOSTSYNC|";

/// What a change line says is about to happen. Coarse on purpose: the itemize
/// string is kept beside it, while an agent branches on a small stable set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Update,
    Delete,
    Directory,
    Skip,
    Other,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Directory => "directory",
            Self::Skip => "skip",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub action: Action,
    pub path: String,
    pub itemize: String,
}

/// Turns rsync's stdout into structured changes. Lines without the marker are
/// returned separately so a human can still see what the tool said.
pub fn parse_changes(output: &str) -> (Vec<Change>, Vec<String>) {
    let mut changes = Vec::new();
    let mut other = Vec::new();
    for line in output.replace("\r\n", "\n").split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix(CHANGE_MARKER) {
            // The itemize column is fixed-width text with no "|" in it, so the
            // first separator is the only one to split on: everything after it
            // is the filename, "|" characters and all.
            match rest.split_once('|') {
                Some((itemize, name)) => changes.push(Change {
                    action: classify(itemize),
                    path: name.to_string(),
                    itemize: itemize.to_string(),
                }),
                None => other.push(line.to_string()),
            }
            continue;
        }
        if let Some(name) = line.strip_prefix("*deleting") {
            changes.push(Change {
                action: Action::Delete,
                path: name.trim().to_string(),
                itemize: "*deleting".to_string(),
            });
            continue;
        }
        other.push(line.to_string());
    }
    (changes, other)
}

/// Maps an itemize string to an [`Action`]. Only the leading characters are
/// load-bearing: the second character names the file type, and a run of `+` in
/// the transfer column means "created" while letters mean "something changed".
fn classify(itemize: &str) -> Action {
    let bytes = itemize.as_bytes();
    if itemize.starts_with("*deleting") {
        return Action::Delete;
    }
    if bytes.len() > 1 && bytes[1] == b'd' {
        return Action::Directory;
    }
    if bytes.len() > 1 && b"Lcsbp-".contains(&bytes[1]) {
        // A symlink, socket, device or special file is neither copied nor
        // followed, so it is reported as skipped rather than looking missing.
        return Action::Skip;
    }
    if bytes.len() > 2 && itemize.starts_with(">f") {
        return if itemize[2..].trim_matches('+').is_empty() {
            Action::Create
        } else {
            Action::Update
        };
    }
    if bytes.len() > 2 && itemize.starts_with("cf") {
        return Action::Create;
    }
    Action::Other
}

/// The local name a remote path ends in, used when a `get` lands inside an
/// existing directory and the result must name the file that was created.
pub fn path_base(remote: &str) -> String {
    let remote = match split_remote_spec(remote) {
        Some((_, path)) => path,
        None => remote,
    };
    let remote = remote.trim_end_matches('/');
    match remote.rsplit_once('/') {
        Some((_, name)) if !name.is_empty() => name.to_string(),
        _ if !remote.is_empty() => remote.to_string(),
        _ => "rhost-download".to_string(),
    }
}

/// The `mkdir -p` command that creates a remote file's parent directory, or
/// `None` when the path has no parent to create.
pub fn remote_parent_command(remote: &str) -> Option<String> {
    let trimmed = remote.trim_end_matches('/');
    let parent = if remote.ends_with('/') {
        trimmed
    } else {
        match trimmed.rsplit_once('/') {
            Some(("", _)) => "",
            Some((head, _)) => head,
            None => "",
        }
    };
    if parent.is_empty() || parent == "." {
        return None;
    }
    Some(format!("mkdir -p -- {}", crate::shell::path_quote(parent)))
}

/// True when a remote path would be re-parsed by the remote shell, so rsync
/// needs `--protect-args` (or a hand-quoted path) to keep it one literal.
pub fn path_needs_quoting(path: &str) -> bool {
    path.chars()
        .any(|c| !(c.is_ascii_alphanumeric() || c.is_alphabetic() || "/._~-".contains(c)))
}
