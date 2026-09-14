//! What every helper program starts with: the remote state root, the two
//! preflight checks, and the shell functions the programs share.
//!
//! These are the parts that must not drift between programs — a `create` that
//! resolved a different state root than `exec` would produce a session nobody
//! could find again.

use crate::transport::protocol::DEFAULT_REMOTE_STATE_DIR;

/// The remote state root every script resolves the same way. `RHOST_REMOTE_STATE`
/// overrides it; the default is this major version's own directory, so a session
/// created by another version is neither adopted nor deleted (PERSIST-002).
pub(super) fn preamble() -> String {
    format!("BASE=\"${{RHOST_REMOTE_STATE:-{DEFAULT_REMOTE_STATE_DIR}}}\"\n")
}

/// Fails fast when the remote lacks tmux: without it, `tmux new-session` would
/// fail silently and the readiness wait would surface a misleading answer about
/// the session roughly 30 seconds later.
pub(super) const TMUX_PREFLIGHT: &str =
    "command -v tmux >/dev/null 2>&1 || { echo RHOST_ERR=notmux; exit 0; }\n";

/// Distinguishes a missing `flock` from real lock contention: `flock` failing with
/// 127 would otherwise be reported as "another writer holds the lock", which is a
/// lie.
pub(super) const FLOCK_PREFLIGHT: &str =
    "command -v flock >/dev/null 2>&1 || { echo RHOST_ERR=noflock; exit 0; }\n";

/// Reads the session name out of a `meta.json`.
pub(super) const NAME_OF_FUNC: &str = r#"RHOST_NAME_OF() {
  sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1"
}
"#;

/// Maps a caller-supplied name or id to a session directory. Resolution happens on
/// the remote side so every command stays one SSH round trip.
pub(super) const RESOLVE_FUNC: &str = r#"RHOST_RESOLVE() {
  local want="$1" d id nm
  for d in "$BASE"/sessions/*/; do
    [ -f "$d/meta.json" ] || continue
    id=$(basename "$d")
    nm=$(sed -n 's/.*"name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$d/meta.json")
    if [ "$id" = "$want" ] || { [ -n "$nm" ] && [ "$nm" = "$want" ]; }; then
      RHOST_DIR="$d"
      RHOST_ID="$id"
      RHOST_TMUX=$(sed -n 's/.*"tmux_session"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$d/meta.json")
      return 0
    fi
  done
  return 1
}
"#;

/// Reads the managed shell out of a session's metadata, so the foreground check
/// compares against what this session was created with rather than a hardcoded
/// name.
pub(super) const SHELL_OF_FUNC: &str = r#"RHOST_SHELL_OF() {
  sed -n 's/.*"shell"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1"
}
"#;

/// The incremental completion-marker scan.
///
/// The load-bearing property is the order: measure the log size, read exactly that
/// window, then advance to it. Reading first and measuring afterwards can jump the
/// offset past bytes nobody read, and a marker inside that band is then inside no
/// later window — a finished command would look like a timeout. The `floor` clamp
/// is what lets the window overlap backwards by one marker length, so a marker
/// straddling two polls is still seen without ever reaching into a previous
/// command's marker and reporting its exit status as this one's.
pub(super) const MARKER_SCAN_FUNC: &str = r#"rh_window() {
  cur=$(wc -c < "$LOG" 2>/dev/null || echo 0)
  from=$((scan - dlen + 1))
  [ "$from" -lt "$floor" ] && from=$floor
  if [ "$cur" -ge "$from" ]; then
    tail -c +"$from" "$LOG" 2>/dev/null | head -c $((cur - from + 1)) | grep -aqF "$dpat" && { scan=$cur; return 0; }
  fi
  scan=$cur
  return 1
}
"#;
