//! The one base64 implementation in the crate.
//!
//! Two things cross the wire encoded this way: the wrapper script `ssh` is
//! handed, and the fields a session helper returns. Neither end is a foreign
//! protocol, so the encoding is standard RFC 4648 with padding, and a malformed
//! field is a failure rather than silently decoded garbage.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let word = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for (index, shift) in [(0usize, 18u32), (1, 12), (2, 6), (3, 0)] {
            // A chunk of n source bytes produces n+1 symbols; the rest is pad.
            out.push(if index < chunk.len() + 1 {
                ALPHABET[((word >> shift) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    out
}

/// Decodes one padded standard-alphabet field.
///
/// Newlines are ignored because a helper's output is line-oriented and a long
/// value may be wrapped; everything else is strict, including the padding bits,
/// so two different strings can never decode to the same bytes and a truncated
/// field cannot be mistaken for a short one.
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let clean: Vec<u8> = text
        .bytes()
        .filter(|byte| *byte != b'\r' && *byte != b'\n')
        .collect();
    if clean.is_empty() {
        return Some(Vec::new());
    }
    if clean.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for (group, chunk) in clean.chunks(4).enumerate() {
        let last = group == clean.len() / 4 - 1;
        let pad = chunk.iter().filter(|byte| **byte == b'=').count();
        if pad > 0 && !last {
            return None;
        }
        if pad > 2 || chunk[..4 - pad].contains(&b'=') {
            return None;
        }
        let mut word = 0u32;
        for (index, byte) in chunk.iter().enumerate() {
            let value = match byte {
                b'=' if index >= 2 => 0,
                _ => u32::try_from(ALPHABET.iter().position(|c| c == byte)?).ok()?,
            };
            word |= value << (18 - 6 * index);
        }
        // The bits a padding character stands in for must be zero.
        if pad == 1 && word & 0xff != 0 {
            return None;
        }
        if pad == 2 && word & 0xffff != 0 {
            return None;
        }
        out.push((word >> 16) as u8);
        if pad < 2 {
            out.push((word >> 8) as u8);
        }
        if pad < 1 {
            out.push(word as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_alphabet_and_padding_round_trip() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"hello world\n"), "aGVsbG8gd29ybGQK");
        for case in [&b""[..], b"f", b"fo", b"foo", b"hello world\n"] {
            let text = encode(case);
            assert_eq!(decode(&text).as_deref(), Some(case), "{text}");
        }
    }

    #[test]
    fn a_wrapped_or_padded_field_decodes_and_the_rest_is_refused() {
        assert_eq!(
            decode("aGVsbG8g\n\nd29ybGQ=").as_deref(),
            Some(&b"hello world"[..])
        );
        assert_eq!(decode("").as_deref(), Some(&b""[..]));
        // Not a length a padded group can have.
        assert!(decode("Zg=").is_none());
        // Padding in the middle, or more than one group of it.
        assert!(decode("Zg==Zg==").is_none());
        assert!(decode("Z===").is_none());
        // Non-zero bits under the padding, so two strings would share one value.
        assert!(decode("Zh==").is_none());
        // An alphabet character outside the standard set.
        assert!(decode("Zg*=").is_none());
    }
}
