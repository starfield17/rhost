//! Random material from the operating system.
//!
//! Everything rhost needs randomness for is an identifier: a tunnel id, a
//! session id, and the per-submission token that binds completion evidence to one
//! invocation. None of them may be guessable, so they come from the same source
//! and are never derived from time, a pid, or a counter.

use std::fs::File;
use std::io::{self, Read};

/// `bytes` bytes as lowercase hex.
pub(crate) fn hex(bytes: usize) -> io::Result<String> {
    let mut raw = vec![0u8; bytes];
    File::open("/dev/urandom")?.read_exact(&mut raw)?;
    let mut out = String::with_capacity(bytes * 2);
    for byte in raw {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_call_is_hex_of_the_requested_length_and_differs() {
        let first = hex(16).unwrap_or_else(|error| panic!("{error}"));
        let second = hex(16).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(first.len(), 32);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "{first}"
        );
        assert_ne!(first, second);
        assert_eq!(hex(0).unwrap_or_default(), "");
    }
}
