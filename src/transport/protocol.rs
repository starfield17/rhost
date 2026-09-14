//! The remote execution protocol: one wrapper script that runs a command in its
//! own session/process group, records its pid so a timed-out command can be
//! stopped remotely, and prints a nonce-bound completion marker carrying the
//! real exit status.
//!
//! The marker is *evidence*, not a handshake: it means something only because
//! the nonce is unguessable and is checked against the invocation that submitted
//! it (EXEC-004). Nothing here parses another implementation's marker.

mod stream;

use crate::domain::InvocationToken;
use std::io;

pub use stream::CompletionStream;

pub(crate) const MARKER_PREFIX: &str = "__RHOST_DONE_";
pub(crate) const BEGIN_PREFIX: &str = "__RHOST_BEGIN_";

/// The remote per-user state root, as a shell expression so the host can override
/// it with `RHOST_REMOTE_STATE`.
///
/// It is this major version's own directory: session records and the wrapper's
/// pid files are state, and a v4 binary must not adopt or delete what a v3
/// installation left behind (CONTRACT.md PERSIST-002).
pub const DEFAULT_REMOTE_STATE_DIR: &str = "$HOME/.local/state/rhost/v4";

/// How markers are separated from command output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    /// A captured run: the marker is the last line, so a newline boundary is
    /// enough.
    Newline,
    /// A streamed run: a NUL boundary means the parser never has to hold back a
    /// command's final newline while deciding whether a marker follows.
    Nul,
}

impl Separator {
    /// The shell escape spelling emitted inside `printf`'s single-quoted format.
    fn printf(self) -> &'static str {
        match self {
            Self::Newline => r"\n",
            Self::Nul => r"\000",
        }
    }

    fn byte(self) -> u8 {
        match self {
            Self::Newline => b'\n',
            Self::Nul => 0,
        }
    }
}

pub fn marker_token(nonce: &InvocationToken) -> String {
    format!("{MARKER_PREFIX}{}__", nonce.as_str())
}

pub fn begin_token(nonce: &InvocationToken) -> String {
    format!("{BEGIN_PREFIX}{}__", nonce.as_str())
}

/// The byte needle a reader searches for: separator, token, and for completion
/// the `:` that must follow the nonce.
pub fn done_needle(separator: Separator, nonce: &InvocationToken) -> Vec<u8> {
    let mut needle = vec![separator.byte()];
    needle.extend(marker_token(nonce).into_bytes());
    needle.push(b':');
    needle
}

/// The begin needle includes the newline the wrapper prints after it: bytes up
/// to and including this needle are login-profile noise, never command output.
pub fn begin_needle(separator: Separator, nonce: &InvocationToken) -> Vec<u8> {
    let mut needle = vec![separator.byte()];
    needle.extend(begin_token(nonce).into_bytes());
    needle.push(b'\n');
    needle
}

/// One foreground submission. `env` is exported in sorted key order so the same
/// request always produces the same script.
pub struct ExecSpec<'a> {
    pub command: &'a str,
    pub cwd: Option<&'a str>,
    pub env: &'a [(String, String)],
    pub nonce: &'a InvocationToken,
    pub remote_state_dir: Option<&'a str>,
}

/// A 128-bit random token, strong enough that ordinary command output cannot
/// forge a completion for an invocation it is not part of.
pub fn new_nonce() -> Result<InvocationToken, std::io::Error> {
    let hex = crate::random::hex(16)?;
    InvocationToken::new(hex).map_err(|error| io::Error::other(error.to_string()))
}

/// The bash script executed on the remote host.
///
/// It runs under a login shell (`bash -lc`) inside a new session (`setsid`), so
/// `$$` is the session and process-group leader and the whole command can be
/// stopped with `kill -TERM -$$`. The script always exits 0: the command's real
/// status rides in the completion marker, which keeps the ssh-level exit status
/// free to signal *transport* failure unambiguously.
pub fn build_script(spec: &ExecSpec<'_>, separator: Separator) -> String {
    let nonce = spec.nonce.as_str();
    let boundary = separator.printf();
    let marker = marker_token(spec.nonce);
    let mut script = String::new();
    match spec.remote_state_dir {
        Some(state) => script.push_str(&format!("RHOST_RD={}\n", crate::shell::quote(state))),
        None => script.push_str(&format!(
            "RHOST_RD=\"${{RHOST_REMOTE_STATE:-{DEFAULT_REMOTE_STATE_DIR}}}\"\n"
        )),
    }
    script.push_str("mkdir -p \"$RHOST_RD/run\" 2>/dev/null && RHOST_PD=\"$RHOST_RD/run\" || RHOST_PD=\"${TMPDIR:-/tmp}\"\n");
    script.push_str(&format!("RHOST_PF=\"$RHOST_PD/rhost-{nonce}.pid\"\n"));
    script.push_str("RHOST_PS=$(ps -o lstart= -p \"$$\" 2>/dev/null)\n");
    script.push_str("[ -n \"$RHOST_PS\" ] && printf '%s\\n%s\\n' \"$$\" \"$RHOST_PS\" > \"$RHOST_PF\" 2>/dev/null || RHOST_PF=\"\"\n");
    script.push_str("trap 'rm -f \"$RHOST_PF\"' EXIT\n");
    if let Some(cwd) = spec.cwd {
        let quoted = crate::shell::path_quote(cwd);
        // A directory the remote cannot enter is a completed execution with
        // status 126, carried by the same nonce-bound marker as any other run:
        // it is not a transport failure and not an unknown.
        script.push_str(&format!(
            "cd -- {quoted} 2>/dev/null || {{ printf 'rhost: cannot change directory to %s\\n' {quoted} >&2; printf '{boundary}{marker}:%d\\n' 126; exit 0; }}\n"
        ));
    }
    let mut env = spec.env.to_vec();
    env.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in &env {
        script.push_str(&format!("export {key}={}\n", crate::shell::quote(value)));
    }
    // The begin marker is emitted immediately before the command's output. The
    // wrapper runs under `bash -lc`, which sources the login profile first; a
    // noisy profile's stdout would otherwise be prepended to the command's own
    // output.
    script.push_str(&format!(
        "printf '{boundary}{}\\n'\n",
        begin_token(spec.nonce)
    ));
    // The command runs in a subshell so a bare `exit` inside it cannot skip the
    // completion marker.
    script.push_str("(\n");
    script.push_str(spec.command);
    if !spec.command.ends_with('\n') {
        script.push('\n');
    }
    script.push_str(")\nRHOST_EC=$?\n");
    script.push_str(&format!(
        "printf '{boundary}{marker}:%d\\n' \"$RHOST_EC\"\nexit 0\n"
    ));
    script
}

/// The command string handed to `ssh`.
///
/// Only ASCII base64 crosses the remote account's login-shell parser: fish and
/// POSIX shells disagree about backslashes inside single quotes, and this outer
/// string is parsed by whichever shell the account uses.
pub fn wrap_script(script: &str) -> String {
    let encoded = crate::base64::encode(script.as_bytes());
    let inner = format!("eval \"$(printf %s {encoded} | base64 -d)\"");
    format!("exec setsid bash -lc {}", crate::shell::quote(&inner))
}

/// Splits captured stdout into the command's own output and its completion code.
///
/// Everything up to and including the *first* begin marker is login-profile
/// noise, not the command's output. A missing or unparseable marker yields no
/// evidence: it never becomes a known exit (EXEC-005).
pub fn parse_marker<'a>(stdout: &'a [u8], nonce: &InvocationToken) -> Option<(&'a [u8], i32)> {
    let needle = done_needle(Separator::Newline, nonce);
    let start = find_last(stdout, &needle)?;
    let rest = &stdout[start + needle.len()..];
    let end = rest.iter().position(|byte| *byte == b'\n')?;
    let text = std::str::from_utf8(&rest[..end]).ok()?.trim();
    let code = text.parse::<i32>().ok()?;
    let mut body = &stdout[..start];
    let begin = begin_needle(Separator::Newline, nonce);
    if let Some(noise) = find_first(body, &begin) {
        body = &body[noise + begin.len()..];
    }
    Some((body, code))
}

fn find_first(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Completion markers arrive last, so scanning from the end is the cheap and
/// usual path.
fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// A remote command that stops the process group recorded for `nonce`,
/// escalating TERM then KILL, and prints exactly one verdict.
///
/// Killing the local ssh process does not stop the remote command, so cleanup is
/// a separate submission. A missing pid file, a pid that is no longer the same
/// process, or a group that survives both signals mean the same thing: rhost
/// cannot claim the command stopped (EXEC-007).
pub fn kill_command(nonce: &InvocationToken) -> String {
    let name = nonce.as_str();
    let state = DEFAULT_REMOTE_STATE_DIR;
    format!(
        "f1=\"${{RHOST_REMOTE_STATE:-{state}}}/run/rhost-{name}.pid\"; \
         f2=\"${{TMPDIR:-/tmp}}/rhost-{name}.pid\"; \
         pf=\"$f1\"; [ -f \"$pf\" ] || pf=\"$f2\"; \
         {{ IFS= read -r p; IFS= read -r started; }} < \"$pf\" 2>/dev/null || {{ echo cleanup-unknown; exit 0; }}; \
         case \"$p\" in ''|*[!0-9]*) echo cleanup-unknown; exit 0;; esac; \
         now=$(ps -o lstart= -p \"$p\" 2>/dev/null); pg=$(ps -o pgid= -p \"$p\" 2>/dev/null | tr -d ' '); sid=$(ps -o sid= -p \"$p\" 2>/dev/null | tr -d ' '); \
         [ -n \"$started\" ] && [ \"$now\" = \"$started\" ] && [ \"$pg\" = \"$p\" ] && [ \"$sid\" = \"$p\" ] || {{ echo cleanup-unknown; exit 0; }}; \
         kill -TERM -\"$p\" 2>/dev/null || {{ echo cleanup-unknown; exit 0; }}; \
         i=0; while [ \"$i\" -lt 10 ] && ps -eo pgid= 2>/dev/null | tr -d ' ' | grep -qx \"$p\"; do sleep 0.1; i=$((i+1)); done; \
         if ps -eo pgid= 2>/dev/null | tr -d ' ' | grep -qx \"$p\"; then kill -KILL -\"$p\" 2>/dev/null || {{ echo cleanup-unknown; exit 0; }}; fi; \
         i=0; while [ \"$i\" -lt 10 ] && ps -eo pgid= 2>/dev/null | tr -d ' ' | grep -qx \"$p\"; do sleep 0.1; i=$((i+1)); done; \
         if ps -eo pgid= 2>/dev/null | tr -d ' ' | grep -qx \"$p\"; then echo cleanup-unknown; else rm -f \"$f1\" \"$f2\"; echo cleanup-confirmed; fi"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> InvocationToken {
        InvocationToken::new(value.to_string()).unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn wrapper_records_the_group_and_binds_the_marker() {
        let nonce = token(&"a".repeat(32));
        let env = vec![
            ("ZZ".to_string(), "1".to_string()),
            ("AA".to_string(), "2".to_string()),
        ];
        let script = build_script(
            &ExecSpec {
                command: "printf hi",
                cwd: Some("~/work dir"),
                env: &env,
                nonce: &nonce,
                remote_state_dir: None,
            },
            Separator::Nul,
        );
        assert!(script.starts_with(&format!(
            "RHOST_RD=\"${{RHOST_REMOTE_STATE:-{DEFAULT_REMOTE_STATE_DIR}}}\"\n"
        )));
        assert!(script.contains("rhost-"));
        assert!(script.contains("trap 'rm -f \"$RHOST_PF\"' EXIT\n"));
        assert!(script.contains("cd -- \"$HOME\"/'work dir'"));
        assert!(script.contains("export AA='2'\nexport ZZ='1'\n"));
        assert!(script.contains("\\000__RHOST_BEGIN_"));
        assert!(script.contains("\\000__RHOST_DONE_aaa"));
        assert!(script.ends_with("__:%d\\n' \"$RHOST_EC\"\nexit 0\n"));
        assert!(script.contains("(\nprintf hi\n)\nRHOST_EC=$?\n"));
    }

    #[test]
    fn wrapper_and_cleanup_resolve_the_same_remote_state_override() {
        let nonce = token(&"d".repeat(32));
        let script = build_script(
            &ExecSpec {
                command: "true",
                cwd: None,
                env: &[],
                nonce: &nonce,
                remote_state_dir: None,
            },
            Separator::Newline,
        );
        let expression = format!("${{RHOST_REMOTE_STATE:-{DEFAULT_REMOTE_STATE_DIR}}}");
        assert!(script.contains(&format!("RHOST_RD=\"{expression}\"")));
        assert!(kill_command(&nonce).contains(&format!("f1=\"{expression}/run/")));
    }

    #[test]
    fn captured_marker_is_split_and_foreign_output_is_not_evidence() {
        let nonce = token(&"b".repeat(32));
        let other = token(&"c".repeat(32));
        let stdout = format!(
            // done_needle already ends in the `:` that separates the token from
            // the status, so the status follows it directly.
            "profile noise{}body\n{}7\n",
            needle_text(&begin_needle(Separator::Newline, &nonce)),
            needle_text(&done_needle(Separator::Newline, &nonce)),
        );
        let (body, code) =
            parse_marker(stdout.as_bytes(), &nonce).unwrap_or_else(|| panic!("marker missed"));
        assert_eq!(body, b"body\n");
        assert_eq!(code, 7);
        assert!(parse_marker(b"no marker at all", &nonce).is_none());
        // A marker carrying another invocation's nonce is ordinary output.
        let foreign = format!(
            "{}0\n",
            needle_text(&done_needle(Separator::Newline, &other))
        );
        assert!(parse_marker(foreign.as_bytes(), &nonce).is_none());
        assert!(
            parse_marker(
                format!(
                    "{}nope\n",
                    needle_text(&done_needle(Separator::Newline, &other))
                )
                .as_bytes(),
                &other,
            )
            .is_none()
        );
        // A marker whose status line never finished is not evidence either.
        assert!(parse_marker(&done_needle(Separator::Newline, &other), &other).is_none());
    }

    fn needle_text(needle: &[u8]) -> String {
        String::from_utf8(needle.to_vec()).unwrap_or_else(|error| panic!("{error}"))
    }

    #[test]
    fn only_ascii_base64_crosses_the_login_shell() {
        let wrapped = wrap_script("echo 'quoted'\n");
        assert!(wrapped.starts_with("exec setsid bash -lc 'eval \"$(printf %s "));
        assert!(wrapped.contains(" | base64 -d)\"'"));
        assert!(wrapped.bytes().all(|byte| byte.is_ascii()));
    }
}
