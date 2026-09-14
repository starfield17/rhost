//! Quoting and byte handling for values that cross a remote shell boundary.
//!
//! There is exactly one implementation of this logic in the crate
//! (docs/architecture/runtime.md): any caller that needs a remote shell to see a
//! value as *one literal* goes through here.

/// POSIX single-quoted form of `s`.
///
/// The `'\''` escape idiom is understood by sh/bash/dash/ksh/zsh and also by
/// fish, which matters because sshd runs the client command through the remote
/// account's *login* shell.
pub fn quote(value: &str) -> String {
    if value.is_empty() {
        return String::from("''");
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Quotes a remote path as one literal value, expanding only the current remote
/// user's leading home shorthand. Expansion happens on the remote host, never
/// here (FS-001).
pub fn path_quote(value: &str) -> String {
    if value == "~" {
        return String::from("\"$HOME\"");
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return format!("\"$HOME\"/{}", quote(rest));
    }
    quote(value)
}

/// Accepts only POSIX environment-variable identifiers. This closes the
/// `export FOO=bar\necho pwned` injection class.
pub fn is_valid_env_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Trailing bytes of `b` that form an incomplete UTF-8 sequence, or 0 when `b`
/// ends on a rune boundary. Used to cut a capture without emitting a partial
/// rune (EXEC-009).
pub fn incomplete_utf8_suffix(bytes: &[u8]) -> usize {
    for index in 1..=4usize.min(bytes.len()) {
        let byte = bytes[bytes.len() - index];
        if byte < 0x80 {
            return 0;
        }
        if byte & 0xC0 == 0xC0 {
            let want = match byte {
                b if b & 0xE0 == 0xC0 => 2,
                b if b & 0xF0 == 0xE0 => 3,
                b if b & 0xF8 == 0xF0 => 4,
                _ => 1,
            };
            return if want > index { index } else { 0 };
        }
    }
    0
}

/// Cuts `bytes` to at most `limit` source bytes, never mid-rune.
pub fn truncate_on_char_boundary(mut bytes: Vec<u8>, limit: usize) -> Vec<u8> {
    if bytes.len() <= limit {
        return bytes;
    }
    let mut cut = limit;
    cut -= incomplete_utf8_suffix(&bytes[..cut]);
    bytes.truncate(cut);
    bytes
}

/// Removes the terminal control sequences rhost's own pane produces and
/// normalises line endings, so captured session output is readable.
///
/// Only two families matter: OSC (the OSC 133 command-boundary markers and
/// terminal titles, ended by BEL or `ESC \`) and CSI (colours, cursor movement,
/// the bracketed-paste toggle). OSC is stripped first so its BEL terminator is
/// not left behind, and a sequence without its terminator is *not* stripped —
/// guessing where an unterminated escape ends would eat real output.
pub fn strip_ansi(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b {
            match bytes.get(index + 1) {
                Some(b']') => {
                    if let Some(end) = osc_end(bytes, index + 2) {
                        index = end;
                        continue;
                    }
                }
                Some(b'[') => {
                    if let Some(end) = csi_end(bytes, index + 2) {
                        index = end;
                        continue;
                    }
                }
                _ => {}
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out)
        .replace("\r\n", "\n")
        .replace('\r', "")
}

/// One character past an OSC string, or `None` when it never terminates.
fn osc_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index < bytes.len() {
        match bytes[index] {
            0x07 => return Some(index + 1),
            0x1b if bytes.get(index + 1) == Some(&b'\\') => return Some(index + 2),
            _ => index += 1,
        }
    }
    None
}

/// One character past a CSI sequence, or `None` when it never reaches a final
/// byte. Parameters are what rhost's terminal emits; anything else is left
/// alone rather than swallowed.
fn csi_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    while matches!(bytes.get(index), Some(b'0'..=b'9' | b';' | b'?')) {
        index += 1;
    }
    match bytes.get(index) {
        Some(byte) if byte.is_ascii_alphabetic() => Some(index + 1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_keeps_one_literal() {
        assert_eq!(quote(""), "''");
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(quote("a $(b) `c` ; d"), "'a $(b) `c` ; d'");
        assert_eq!(path_quote("~"), "\"$HOME\"");
        assert_eq!(path_quote("~/a b"), "\"$HOME\"/'a b'");
        assert_eq!(path_quote("/tmp/x"), "'/tmp/x'");
    }

    #[test]
    fn env_keys_and_utf8_boundaries() {
        assert!(is_valid_env_key("_A9"));
        assert!(!is_valid_env_key("9A"));
        assert!(!is_valid_env_key("A B"));
        assert!(!is_valid_env_key("A=1"));
        assert_eq!(incomplete_utf8_suffix(b"ab\x00"), 0);
        assert_eq!(incomplete_utf8_suffix("é".as_bytes()), 0);
        assert_eq!(incomplete_utf8_suffix(&"é".as_bytes()[..1]), 1);
        // Cutting inside a rune holds the incomplete sequence back rather than
        // emitting half a character; the byte count still describes the source.
        assert_eq!(
            truncate_on_char_boundary("é".as_bytes().to_vec(), 1),
            Vec::<u8>::new()
        );
        assert_eq!(
            truncate_on_char_boundary(b"abc".to_vec(), 0),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn terminal_sequences_are_stripped_and_line_endings_normalised() {
        assert_eq!(strip_ansi("\x1b]133;C\x07hello\r\n"), "hello\n");
        assert_eq!(strip_ansi("\x1b]133;D;0\x07"), "");
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\after"), "after");
        assert_eq!(
            strip_ansi("\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\"),
            "link"
        );
        assert_eq!(strip_ansi("\x1b[?2004h$ ls\x1b[?2004l\r\n"), "$ ls\n");
        assert_eq!(strip_ansi("plain text\n"), "plain text\n");
        assert_eq!(strip_ansi("a\rb\n"), "ab\n");
        // An unterminated escape is output, not a sequence to guess at.
        assert_eq!(strip_ansi("x\x1b]13"), "x\x1b]13");
        assert_eq!(strip_ansi("x\x1b["), "x\x1b[");
    }
}
