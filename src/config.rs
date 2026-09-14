//! Local filesystem locations owned by rhost.
//!
//! This is deliberately separate from OpenSSH configuration: OpenSSH stays the
//! source of truth for hosts, users, keys and host-key policy (AGENTS.md §5).
//! rhost only needs a private place for its own ControlMaster sockets.
use sha2::{Digest, Sha256};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "rhost";

/// A local cache/state path that cannot safely hold private process metadata or
/// control sockets.
#[derive(Debug)]
pub struct UnsafeLocalState(pub String);
impl std::fmt::Display for UnsafeLocalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsafe local rhost state: {}", self.0)
    }
}
impl std::error::Error for UnsafeLocalState {}

fn environment(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn home_dir() -> Option<PathBuf> {
    environment("HOME")
}

/// Fallback root for the socket directory. Fixed and short on purpose: the
/// per-session temporary root on macOS is itself deep enough to overflow
/// `sun_path`, which is the failure this branch exists to avoid.
const SOCKET_FALLBACK_ROOT: &str = "/tmp";

/// Conservative `sun_path` budget (104 including the NUL on BSD-derived systems;
/// Linux allows 108).
const MAX_SOCKET_PATH: usize = 104;

/// Length OpenSSH substitutes for `%C`.
const HASH_TOKEN_LEN: usize = 40;

/// rhost's local cache root; `RHOST_CACHE_DIR` overrides it.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = environment("RHOST_CACHE_DIR") {
        return dir;
    }
    let base = if cfg!(target_os = "macos") {
        home_dir().map(|home| home.join("Library").join("Caches"))
    } else {
        match environment("XDG_CACHE_HOME") {
            Some(xdg) => Some(xdg),
            None => home_dir().map(|home| home.join(".cache")),
        }
    };
    match base {
        Some(base) => base.join(APP_NAME),
        None => std::env::temp_dir().join("rhost-cache").join(APP_NAME),
    }
}

/// rhost's local state root; this version's own files live under
/// [`v4_state_dir`] inside it. `RHOST_STATE_DIR` overrides it for tests and for a
/// user who wants the state elsewhere.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = environment("RHOST_STATE_DIR") {
        return dir;
    }
    if let Some(xdg) = environment("XDG_STATE_HOME") {
        return xdg.join(APP_NAME);
    }
    if let Some(home) = home_dir() {
        return home.join(".local").join("state").join(APP_NAME);
    }
    std::env::temp_dir().join("rhost-state").join(APP_NAME)
}

/// The state root this major version owns.
///
/// v4 writes under its own namespace so it can neither adopt nor delete what a
/// v3 installation left behind: an old tunnel record here would name a socket
/// this binary never opened (CONTRACT.md PERSIST-002).
pub fn v4_state_dir() -> PathBuf {
    state_dir().join("v4")
}

/// The socket home for an explicit cache root.
///
/// Two shapes of cache root cannot host the sockets, and both move to a short
/// per-cache root: a deep one, because OpenSSH's fixed 40-byte `%C` expansion
/// overflows `sun_path` (ssh then refuses every command with "ControlPath too
/// long", SSH-003); and one containing whitespace, because it cannot ride
/// rsync's `-e` string, which would silently cost a multiplexed transfer its
/// connection reuse.
fn control_dir_in(root: &Path) -> PathBuf {
    let candidate = root.join("ssh");
    if socket_fits(&candidate) && rsh_safe(&candidate) {
        return candidate;
    }
    let digest = Sha256::digest(canonical_token(root));
    let mut name = String::from("rhost-");
    for byte in &digest[..8] {
        name.push_str(&format!("{byte:02x}"));
    }
    Path::new(SOCKET_FALLBACK_ROOT).join(name).join("ssh")
}

/// A stable token for a cache root. The root need not exist yet, so the digest
/// covers the cleaned textual path rather than a resolved one.
fn canonical_token(root: &Path) -> Vec<u8> {
    let text = root.to_string_lossy();
    let trimmed = text.trim_end_matches('/');
    let normalized = if trimmed.is_empty() { "/" } else { trimmed };
    if Path::new(normalized).is_relative() {
        if let Ok(absolute) = std::path::absolute(Path::new(normalized)) {
            return absolute.to_string_lossy().into_owned().into_bytes();
        }
    }
    normalized.as_bytes().to_vec()
}

fn rsh_safe(dir: &Path) -> bool {
    !dir.to_string_lossy()
        .chars()
        .any(|c| c == ' ' || c == '\t' || c == '\n')
}

fn socket_fits(dir: &Path) -> bool {
    let expanded = dir.join("0".repeat(HASH_TOKEN_LEN));
    expanded.to_string_lossy().len() < MAX_SOCKET_PATH
}

pub fn control_dir() -> PathBuf {
    control_dir_in(&cache_dir())
}

/// The OpenSSH `ControlPath` *template*: one socket per target, keyed by `%C`
/// (a hash of local host, remote host, port and user).
pub fn control_path() -> String {
    control_dir().join("%C").to_string_lossy().into_owned()
}

/// Creates the socket directory with user-only permissions, proving ownership
/// rather than assuming it.
pub fn ensure_control_dir() -> Result<(), UnsafeLocalState> {
    let dir = control_dir();
    // The fallback lives in a world-writable root, so the root of the socket
    // directory is created and proved private before the sockets go inside it.
    if dir.starts_with(SOCKET_FALLBACK_ROOT) {
        if let Some(root) = dir
            .parent()
            .filter(|p| *p != Path::new(SOCKET_FALLBACK_ROOT))
        {
            ensure_private_dir(root)
                .map_err(|error| UnsafeLocalState(format!("control root: {error}")))?;
        }
    }
    ensure_private_dir(&dir)
        .map_err(|error| UnsafeLocalState(format!("control directory: {error}")))
}

/// Creates `dir` privately. `chmod` only succeeds for the directory's owner (or
/// root), so a successful mode change is the ownership proof: a pre-existing
/// directory belonging to another user is refused instead of adopted.
pub(crate) fn ensure_private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let info = std::fs::symlink_metadata(dir)?;
    if info.file_type().is_symlink() || !info.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a real directory", dir.display()),
        ));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let mode = std::fs::metadata(dir)?.permissions().mode();
    if mode & 0o7777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not user-only", dir.display()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_and_spaced_cache_roots_move_to_a_short_home() {
        let deep = Path::new("/tmp")
            .join("a".repeat(90))
            .join("Library")
            .join("Caches")
            .join("rhost");
        let moved = control_dir_in(&deep);
        assert!(
            moved.to_string_lossy().len() + 1 + HASH_TOKEN_LEN <= MAX_SOCKET_PATH,
            "{}",
            moved.display()
        );
        let spaced = PathBuf::from("/tmp/two words/cache");
        assert!(!control_dir_in(&spaced).starts_with(&spaced));
        let shallow = PathBuf::from("/tmp/ok-cache");
        assert_eq!(control_dir_in(&shallow), shallow.join("ssh"));
    }

    #[test]
    fn same_root_hashes_to_the_same_fallback_and_differs_across_roots() {
        let deep = |seed: usize| Path::new("/tmp").join("d".repeat(80) + &seed.to_string());
        assert_eq!(control_dir_in(&deep(1)), control_dir_in(&deep(1)));
        assert_ne!(control_dir_in(&deep(1)), control_dir_in(&deep(2)));
    }
}
